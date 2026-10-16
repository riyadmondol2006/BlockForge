package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import org.objectweb.asm.Handle;
import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.InvokeDynamicInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * sound-no-wait (level 2): every time a sound starts, SoundEngine.play asks the sound thread for an OpenAL channel and
 * waits for the answer (ChannelAccess.createHandle(..).join()). The sound thread also decodes music and long sounds,
 * so the game's main thread can stand still until it gets there: the analysis measured waits of up to 0.5 s (A2 #7).
 *
 * The patch hands out the channel handle at once and lets the sound thread fill in its channel later. The sound
 * thread runs its tasks in order, so the channel is there before any later task of that handle (pitch, volume,
 * buffer, play) runs:
 * <pre>
 *   createHandle(pool):  future = new CompletableFuture();
 *                        + blockforge$lite$ready(this, future);        // completes it with new ChannelHandle(this, null)
 *                        executor.execute(() -> lambda(pool, future));
 *   lambda (sound thread): channel = library.acquireChannel(pool);
 *                        if (channel != null) {
 *                            handle = new ChannelHandle(this, channel);     becomes   blockforge$lite$fill(this, future, channel)
 *                            channels.add(handle); future.complete(handle);  (the ready handle gets the channel)
 *                        } else
 *                            future.complete(null);                         becomes   future.complete(blockforge$lite$none(future))
 *                                                                           (the ready handle is released: marked stopped)
 * </pre>
 * The difference to vanilla: when all channels are taken (a rare case), the sound still counts as started for a
 * second (it is never heard, as before) instead of "not started"; for music that can also mean a "now playing"
 * notice for a song that is not heard. Sound mods that use the channel handle themselves (Sound Physics Remastered,
 * AmbientSounds, Presence Footsteps) switch it off.
 *
 * Applies to Minecraft 1.17 - 26.x class files (the handle's fields are set from ChannelAccess, which needs the nest
 * of Java 11+ class files); checked 1.20.1 - 26.4-snapshot-4.
 */
final class SoundNoWait extends Patch {
    static final String FUTURE = "java/util/concurrent/CompletableFuture";
    static final String READY = Code.PREFIX + "ready";
    static final String FILL = Code.PREFIX + "fill";
    static final String NONE = Code.PREFIX + "none";

    SoundNoWait() {
        super("sound-no-wait", 2, "starting a sound doesn't make the game wait for the sound thread (no short freezes; "
                + "when all sound channels are busy a sound counts as playing for a second, silent as before)",
                new String[][] {{FUTURE, "complete", "java/util/concurrent/Executor", "java/util/Set"}},
                new String[] {"soundphysics", "ambientsounds", "presencefootsteps"},
                "net/minecraft/client/sounds/ChannelAccess", "net/minecraft/class_4235");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        // createHandle: new CompletableFuture; astore f; executor.execute(lambda(this, pool, f)); return f
        MethodNode create = null;
        InvokeDynamicInsnNode indy = null;
        for (Object[] site : t.calls(Opcodes.INVOKEINTERFACE, "java/util/concurrent/Executor", "execute",
                "(Ljava/lang/Runnable;)V")) {
            MethodNode m = (MethodNode) site[0];
            Type[] args = Type.getArgumentTypes(m.desc);
            if (!Type.getReturnType(m.desc).getDescriptor().equals("L" + FUTURE + ";") || args.length != 1
                    || args[0].getSort() != Type.OBJECT || (m.access & Opcodes.ACC_STATIC) != 0) {
                continue;
            }
            if (create != null) {
                throw Skip.because("two methods look like createHandle");
            }
            create = m;
            AbstractInsnNode prev = Code.prev((AbstractInsnNode) site[1]);
            indy = prev instanceof InvokeDynamicInsnNode ? (InvokeDynamicInsnNode) prev : null;
        }
        // ChannelAccess keeps its handles in a Set and runs everything on an Executor (other classes with a method
        // like createHandle, such as resource reload helpers, have neither)
        if (create == null || t.fieldsOfType("Ljava/util/Set;") != 1
                || t.fieldsOfType("Ljava/util/concurrent/Executor;") != 1) {
            throw Skip.notTarget("no createHandle");
        }
        if (t.version < Opcodes.V11) {
            throw Skip.because("class file older than Java 11 (no nest of the handle class)");
        }
        int[] want = {Opcodes.NEW, Opcodes.DUP, Opcodes.INVOKESPECIAL, Opcodes.ASTORE, Opcodes.ALOAD, Opcodes.GETFIELD,
            Opcodes.ALOAD, Opcodes.ALOAD, Opcodes.ALOAD, Opcodes.INVOKEDYNAMIC, Opcodes.INVOKEINTERFACE, Opcodes.ALOAD,
            Opcodes.ARETURN};
        List<AbstractInsnNode> body = real(create);
        if (body.size() != want.length || indy == null || Code.changedByMixin(create)) {
            throw Skip.because("createHandle looks different");
        }
        for (int i = 0; i < want.length; i++) {
            if (body.get(i).getOpcode() != want[i]) {
                throw Skip.because("createHandle looks different");
            }
        }
        AbstractInsnNode store = body.get(3);
        int future = ((VarInsnNode) store).var;
        if (!((TypeInsnNode) body.get(0)).desc.equals(FUTURE) || !Code.isVar(body.get(8), Opcodes.ALOAD, future)
                || !Code.isVar(body.get(11), Opcodes.ALOAD, future) || !Code.isVar(body.get(6), Opcodes.ALOAD, 0)
                || !Code.isVar(body.get(7), Opcodes.ALOAD, 1)) {
            throw Skip.because("createHandle looks different");
        }

        // the lambda the executor runs
        if (indy.bsmArgs.length < 2 || !(indy.bsmArgs[1] instanceof Handle)
                || !indy.bsm.getOwner().equals("java/lang/invoke/LambdaMetafactory")) {
            throw Skip.because("createHandle's task is not a plain lambda");
        }
        Handle impl = (Handle) indy.bsmArgs[1];
        Type pool = Type.getArgumentTypes(create.desc)[0];
        if (!impl.getOwner().equals(t.name)
                || !impl.getDesc().equals("(" + pool.getDescriptor() + "L" + FUTURE + ";)V")) {
            throw Skip.because("createHandle's task is not the expected lambda");
        }
        MethodNode lambda = t.method(impl.getName(), impl.getDesc());
        if (lambda == null || (lambda.access & Opcodes.ACC_STATIC) != 0 || Code.changedByMixin(lambda)
                || !lambda.tryCatchBlocks.isEmpty()) {
            throw Skip.because("createHandle's task is missing or changed");
        }
        Lambda l = Lambda.read(t, lambda);
        if (l == null) {
            throw Skip.because("the channel task looks different");
        }

        // the handle's channel field and its release(), as ChannelAccess's other code uses them
        String channelField = null;
        String release = null;
        for (MethodNode m : t.allMethods()) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (n instanceof FieldInsnNode && ((FieldInsnNode) n).owner.equals(l.handle)
                        && ((FieldInsnNode) n).desc.equals("L" + l.channel + ";")) {
                    String f = ((FieldInsnNode) n).name;
                    if (channelField != null && !channelField.equals(f)) {
                        throw Skip.because("two channel fields");
                    }
                    channelField = f;
                }
                AbstractInsnNode[] then = Code.after(n, Opcodes.ALOAD, Opcodes.INVOKEINTERFACE);
                if (Code.isCall(n, Opcodes.INVOKEVIRTUAL, l.handle, null, "()V") && then != null
                        && Code.isCall(then[1], Opcodes.INVOKEINTERFACE, "java/util/Iterator", "remove", "()V")) {
                    String r = ((MethodInsnNode) n).name;
                    if (release != null && !release.equals(r)) {
                        throw Skip.because("two ways to release a handle");
                    }
                    release = r;
                }
            }
        }
        if (channelField == null || release == null) {
            throw Skip.because("the handle's channel or release() is not used the expected way");
        }

        MethodNode ready = ready(t.name, l);
        MethodNode fill = fill(t.name, l, channelField);
        MethodNode none = none(l, release);
        t.add(ready);
        t.add(fill);
        t.add(none);
        InsnList pre = new InsnList();
        pre.add(new VarInsnNode(Opcodes.ALOAD, 0));
        pre.add(new VarInsnNode(Opcodes.ALOAD, future));
        pre.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, ready.name, ready.desc, false));
        create.instructions.insert(store, pre);
        create.maxStack = Math.max(create.maxStack, 2);
        // new Handle; dup; aload_0; aload channel; invokespecial  ->  aload_0; aload future; aload channel; fill
        InsnList filled = new InsnList();
        filled.add(new VarInsnNode(Opcodes.ALOAD, 0));
        filled.add(new VarInsnNode(Opcodes.ALOAD, l.future));
        filled.add(new VarInsnNode(Opcodes.ALOAD, l.channelVar));
        filled.add(new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, fill.name, fill.desc, false));
        lambda.instructions.insertBefore(l.make[0], filled);
        for (AbstractInsnNode n : l.make) {
            lambda.instructions.remove(n);
        }
        // future.complete(null)  ->  future.complete(none(future))
        lambda.instructions.insertBefore(l.nul, new VarInsnNode(Opcodes.ALOAD, l.future));
        lambda.instructions.set(l.nul, new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, none.name, none.desc, false));
        t.changed(create);
        t.changed(lambda);
    }

    /** The real instructions of a method. */
    private static List<AbstractInsnNode> real(MethodNode m) {
        List<AbstractInsnNode> l = new ArrayList<AbstractInsnNode>();
        for (AbstractInsnNode n = Code.first(m); n != null; n = Code.next(n)) {
            l.add(n);
        }
        return l;
    }

    /** The parts of the channel task that the patch changes, after checking its whole shape. */
    static final class Lambda {
        String handle;
        String channel;
        String handleInit;
        int future;
        int channelVar;
        AbstractInsnNode[] make;
        AbstractInsnNode nul;

        /**
         * aload_0; getfield library; aload_1; invokevirtual acquire(Pool)Channel; astore C; aload C; ifnull NULL;
         * new H; dup; aload_0; aload C; invokespecial H(This, Channel); astore h; aload_0; getfield channels;
         * aload h; invokeinterface Set.add; pop; aload_2; aload h; invokevirtual complete; pop; goto END;
         * NULL: aload_2; aconst_null; invokevirtual complete; pop; END: return
         */
        static Lambda read(Target t, MethodNode m) {
            List<AbstractInsnNode> b = real(m);
            int[] want = {Opcodes.ALOAD, Opcodes.GETFIELD, Opcodes.ALOAD, Opcodes.INVOKEVIRTUAL, Opcodes.ASTORE,
                Opcodes.ALOAD, Opcodes.IFNULL, Opcodes.NEW, Opcodes.DUP, Opcodes.ALOAD, Opcodes.ALOAD,
                Opcodes.INVOKESPECIAL, Opcodes.ASTORE, Opcodes.ALOAD, Opcodes.GETFIELD, Opcodes.ALOAD,
                Opcodes.INVOKEINTERFACE, Opcodes.POP, Opcodes.ALOAD, Opcodes.ALOAD, Opcodes.INVOKEVIRTUAL, Opcodes.POP,
                Opcodes.GOTO, Opcodes.ALOAD, Opcodes.ACONST_NULL, Opcodes.INVOKEVIRTUAL, Opcodes.POP, Opcodes.RETURN};
            if (b.size() != want.length) {
                return null;
            }
            for (int i = 0; i < want.length; i++) {
                if (b.get(i).getOpcode() != want[i]) {
                    return null;
                }
            }
            Lambda l = new Lambda();
            MethodInsnNode acquire = (MethodInsnNode) b.get(3);
            Type ret = Type.getReturnType(acquire.desc);
            if (ret.getSort() != Type.OBJECT || !Code.isVar(b.get(0), Opcodes.ALOAD, 0) || !Code.isVar(b.get(2),
                    Opcodes.ALOAD, 1)) {
                return null;
            }
            l.channel = ret.getInternalName();
            l.channelVar = ((VarInsnNode) b.get(4)).var;
            l.handle = ((TypeInsnNode) b.get(7)).desc;
            MethodInsnNode init = (MethodInsnNode) b.get(11);
            l.handleInit = init.desc;
            int h = ((VarInsnNode) b.get(12)).var;
            l.future = ((VarInsnNode) b.get(18)).var;
            if (!Code.isVar(b.get(5), Opcodes.ALOAD, l.channelVar) || !Code.isVar(b.get(9), Opcodes.ALOAD, 0)
                    || !Code.isVar(b.get(10), Opcodes.ALOAD, l.channelVar) || !init.owner.equals(l.handle)
                    || !init.name.equals("<init>") || !init.desc.equals("(L" + t.name + ";L" + l.channel + ";)V")
                    || !Code.isVar(b.get(13), Opcodes.ALOAD, 0) || !Code.isVar(b.get(15), Opcodes.ALOAD, h)
                    || !Code.isCall(b.get(16), Opcodes.INVOKEINTERFACE, "java/util/Set", "add",
                            "(Ljava/lang/Object;)Z")
                    || !Code.isVar(b.get(19), Opcodes.ALOAD, h) || !isComplete(b.get(20))
                    || !Code.isVar(b.get(23), Opcodes.ALOAD, l.future) || !isComplete(b.get(25)) || l.future != 2) {
                return null;
            }
            l.make = new AbstractInsnNode[] {b.get(7), b.get(8), b.get(9), b.get(10), b.get(11)};
            l.nul = b.get(24);
            return l;
        }

        private static boolean isComplete(AbstractInsnNode n) {
            return Code.isCall(n, Opcodes.INVOKEVIRTUAL, FUTURE, "complete", "(Ljava/lang/Object;)Z");
        }
    }

    /** static void ready(ChannelAccess a, CompletableFuture f) { f.complete(new ChannelHandle(a, null)); } */
    static MethodNode ready(String owner, Lambda l) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, READY,
                "(L" + owner + ";L" + FUTURE + ";)V", null, null);
        h.visitCode();
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitTypeInsn(Opcodes.NEW, l.handle);
        h.visitInsn(Opcodes.DUP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitInsn(Opcodes.ACONST_NULL);
        h.visitMethodInsn(Opcodes.INVOKESPECIAL, l.handle, "<init>", l.handleInit, false);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, FUTURE, "complete", "(Ljava/lang/Object;)Z", false);
        h.visitInsn(Opcodes.POP);
        h.visitInsn(Opcodes.RETURN);
        h.visitMaxs(5, 2);
        h.visitEnd();
        return h;
    }

    /**
     * static ChannelHandle fill(ChannelAccess a, CompletableFuture f, Channel c): the ready handle with its channel
     * set, or (if the future holds no handle) new ChannelHandle(a, c) as before.
     */
    static MethodNode fill(String owner, Lambda l, String channelField) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, FILL,
                "(L" + owner + ";L" + FUTURE + ";L" + l.channel + ";)L" + l.handle + ";", null, null);
        Label fresh = new Label();
        h.visitCode();
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitInsn(Opcodes.ACONST_NULL);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, FUTURE, "getNow", "(Ljava/lang/Object;)Ljava/lang/Object;", false);
        h.visitVarInsn(Opcodes.ASTORE, 3);
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitTypeInsn(Opcodes.INSTANCEOF, l.handle);
        h.visitJumpInsn(Opcodes.IFEQ, fresh);
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitTypeInsn(Opcodes.CHECKCAST, l.handle);
        h.visitVarInsn(Opcodes.ALOAD, 2);
        h.visitFieldInsn(Opcodes.PUTFIELD, l.handle, channelField, "L" + l.channel + ";");
        h.visitVarInsn(Opcodes.ALOAD, 3);
        h.visitTypeInsn(Opcodes.CHECKCAST, l.handle);
        h.visitInsn(Opcodes.ARETURN);
        h.visitLabel(fresh);
        h.visitFrame(Opcodes.F_FULL, 4, new Object[] {owner, FUTURE, l.channel, "java/lang/Object"}, 0, new Object[0]);
        h.visitTypeInsn(Opcodes.NEW, l.handle);
        h.visitInsn(Opcodes.DUP);
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitVarInsn(Opcodes.ALOAD, 2);
        h.visitMethodInsn(Opcodes.INVOKESPECIAL, l.handle, "<init>", l.handleInit, false);
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(4, 4);
        h.visitEnd();
        return h;
    }

    /**
     * static Object none(CompletableFuture f): no channel was free; the ready handle is released (which marks it
     * stopped, so the sound engine drops it; giving back its missing channel throws, which is caught). Returns null,
     * the value vanilla completes the future with.
     */
    static MethodNode none(Lambda l, String release) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, NONE,
                "(L" + FUTURE + ";)Ljava/lang/Object;", null, null);
        Label start = new Label();
        Label end = new Label();
        Label caught = new Label();
        Label done = new Label();
        h.visitCode();
        h.visitTryCatchBlock(start, end, caught, "java/lang/RuntimeException");
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitInsn(Opcodes.ACONST_NULL);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, FUTURE, "getNow", "(Ljava/lang/Object;)Ljava/lang/Object;", false);
        h.visitVarInsn(Opcodes.ASTORE, 1);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitTypeInsn(Opcodes.INSTANCEOF, l.handle);
        h.visitJumpInsn(Opcodes.IFEQ, done);
        h.visitLabel(start);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitTypeInsn(Opcodes.CHECKCAST, l.handle);
        h.visitMethodInsn(Opcodes.INVOKEVIRTUAL, l.handle, release, "()V", false);
        h.visitLabel(end);
        h.visitJumpInsn(Opcodes.GOTO, done);
        h.visitLabel(caught);
        h.visitFrame(Opcodes.F_FULL, 2, new Object[] {FUTURE, "java/lang/Object"}, 1,
                new Object[] {"java/lang/RuntimeException"});
        h.visitInsn(Opcodes.POP);
        h.visitLabel(done);
        h.visitFrame(Opcodes.F_FULL, 2, new Object[] {FUTURE, "java/lang/Object"}, 0, new Object[0]);
        h.visitInsn(Opcodes.ACONST_NULL);
        h.visitInsn(Opcodes.ARETURN);
        h.visitMaxs(2, 2);
        h.visitEnd();
        return h;
    }
}
