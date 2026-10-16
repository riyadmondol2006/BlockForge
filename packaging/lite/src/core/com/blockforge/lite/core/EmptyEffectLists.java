package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.TypeInsnNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * empty-effect-lists: since 1.21.2 every entity that moves collects the effects of the blocks it touches (fire,
 * powder snow, cauldrons...) in InsideBlockEffectApplier.StepBasedCollector. flushStep() runs at every movement step
 * and copies ten per-type lists into the final list, almost always empty ones; ArrayList.addAll still makes a new
 * empty array each time (measured by the analysis, A2b C1: 1.5 - 2.4 % of the server thread, 0.9 MB/s of garbage).
 *
 * The patch replaces both calls with a helper that skips empty lists:
 * <pre>
 *   flushStep():  this.finalEffects.addAll(list);  becomes  blockforge$lite$addAll(this.finalEffects, list);
 *   added:        static boolean blockforge$lite$addAll(List to, Collection from) {
 *                     return from.isEmpty() ? false : to.addAll(from); }
 * </pre>
 * Adding an empty list changes nothing in an ArrayList (it returns false; only its change counter moves, which
 * nothing reads while flushStep runs), so the effects and their order stay exactly the same.
 *
 * Applies to 1.21.2 - 26.x (checked 1.21.11 - 26.4-snapshot-4; the class does not exist in 1.21.1). Lithium,
 * Canary and Radium do the same in their own way; with them the patch stays off.
 */
final class EmptyEffectLists extends Patch {
    static final String HELPER = Code.PREFIX + "addAll";
    static final String ADD_ALL = "(Ljava/util/Collection;)Z";

    EmptyEffectLists() {
        super("empty-effect-lists", 1, "moving entities don't copy empty block-effect lists (less garbage and work on "
                + "the server thread; Minecraft 1.21.2 and newer)",
                new String[][] {{"java/util/EnumSet", "noneOf", "addAll", ADD_ALL}},
                new String[] {"lithium", "canary", "radium"},
                "net/minecraft/world/entity/InsideBlockEffectApplier$StepBasedCollector");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<Object[]> calls = t.calls(Opcodes.INVOKEINTERFACE, "java/util/List", "addAll", ADD_ALL);
        if (calls.isEmpty()) {
            throw Skip.notTarget("no List.addAll");
        }
        MethodNode flush = (MethodNode) calls.get(0)[0];
        String field = null;
        List<AbstractInsnNode> sites = new ArrayList<AbstractInsnNode>();
        for (Object[] site : calls) {
            MethodNode m = (MethodNode) site[0];
            AbstractInsnNode call = (AbstractInsnNode) site[1];
            // this.finalEffects; aload N; addAll; pop; aload N; clear()
            AbstractInsnNode[] before = Code.before(call, Opcodes.ALOAD, Opcodes.GETFIELD, Opcodes.ALOAD);
            AbstractInsnNode[] after = Code.after(call, Opcodes.POP, Opcodes.ALOAD, Opcodes.INVOKEINTERFACE);
            if (m != flush || before == null || after == null) {
                throw Skip.notTarget("addAll used in another way");
            }
            int list = ((VarInsnNode) before[0]).var;
            FieldInsnNode get = (FieldInsnNode) before[1];
            if (!Code.isVar(before[2], Opcodes.ALOAD, 0) || !get.owner.equals(t.name)
                    || !get.desc.equals("Ljava/util/List;") || (field != null && !field.equals(get.name))
                    || !Code.isVar(after[1], Opcodes.ALOAD, list)
                    || !Code.isCall(after[2], Opcodes.INVOKEINTERFACE, "java/util/List", "clear", "()V")) {
                throw Skip.notTarget("addAll used in another way");
            }
            field = get.name;
            sites.add(call);
        }
        if (sites.size() != 2 || !flush.desc.equals("()V") || (flush.access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("expected two List.addAll calls in flushStep, found " + sites.size());
        }
        int access = t.fieldAccess(field, "Ljava/util/List;");
        if (access < 0 || (access & Opcodes.ACC_FINAL) == 0 || (access & Opcodes.ACC_STATIC) != 0
                || !madeAsArrayList(t, field)) {
            throw Skip.because("the final effect list is not a final ArrayList of this class");
        }
        if (Code.changedByMixin(flush)) {
            throw Skip.because("flushStep is changed by a mod");
        }
        MethodNode helper = helper(t.needsFrames());
        t.add(helper);
        for (AbstractInsnNode call : sites) {
            flush.instructions.set(call, new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, helper.name, helper.desc,
                    t.isInterface()));
        }
        t.changed(flush);
    }

    /** The constructors set the field to new ArrayList() and nothing else. */
    private static boolean madeAsArrayList(Target t, String field) {
        int found = 0;
        for (MethodNode m : t.methodsNamed("<init>")) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof FieldInsnNode) || n.getOpcode() != Opcodes.PUTFIELD
                        || !((FieldInsnNode) n).owner.equals(t.name) || !((FieldInsnNode) n).name.equals(field)) {
                    continue;
                }
                AbstractInsnNode[] made = Code.before(n, Opcodes.INVOKESPECIAL, Opcodes.DUP, Opcodes.NEW);
                if (made == null || !Code.isCall(made[0], Opcodes.INVOKESPECIAL, "java/util/ArrayList", "<init>", "()V")
                        || !((TypeInsnNode) made[2]).desc.equals("java/util/ArrayList")) {
                    return false;
                }
                found++;
            }
        }
        return found > 0;
    }

    /** static boolean blockforge$lite$addAll(List to, Collection from) { return !from.isEmpty() &amp;&amp; to.addAll(from); } */
    static MethodNode helper(boolean frames) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, HELPER,
                "(Ljava/util/List;Ljava/util/Collection;)Z", null, null);
        Label add = new Label();
        h.visitCode();
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/Collection", "isEmpty", "()Z", true);
        h.visitJumpInsn(Opcodes.IFEQ, add);
        h.visitInsn(Opcodes.ICONST_0);
        h.visitInsn(Opcodes.IRETURN);
        h.visitLabel(add);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        }
        h.visitVarInsn(Opcodes.ALOAD, 0);
        h.visitVarInsn(Opcodes.ALOAD, 1);
        h.visitMethodInsn(Opcodes.INVOKEINTERFACE, "java/util/List", "addAll", ADD_ALL, true);
        h.visitInsn(Opcodes.IRETURN);
        h.visitMaxs(2, 2);
        h.visitEnd();
        return h;
    }
}
