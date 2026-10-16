package com.blockforge.lite.core;

import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.FieldNode;
import org.objectweb.asm.tree.InsnList;
import org.objectweb.asm.tree.JumpInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * outline-skip: Minecraft 1.20 - 1.21.1 draws the glowing-entity outline layer over the whole screen after every
 * frame in a world (one full-screen blended pass), even when no entity glows and the layer is empty. Minecraft 26.3
 * skips it in that case; this does the same: the picture is identical (blending an empty, fully transparent layer
 * changes no pixel), one full-screen pass per frame less for the graphics chip.
 *
 * Target, LevelRenderer in 1.20 - 1.21.1:
 * <pre>
 *   initOutline():     this.entityTarget = this.entityEffect.getTempTarget("final");
 *   renderLevel(..):   if (glowingEntitiesDrawn) { this.entityEffect.process(..); ... }
 *   doEntityOutline(): if (this.shouldShowEntityOutlines()) { ...; this.entityTarget.blitToScreen(w, h, false); ... }
 * </pre>
 * The patch stores renderLevel's "glowing entities drawn" flag in a static field it adds, and doEntityOutline also
 * asks that field. Because a mod could draw into the outline layer in its own way, the patch is only applied when
 * neither method was changed by a mod or hooked by a mod loader (Forge and NeoForge add event calls there).
 */
final class OutlineSkip extends Patch {
    static final String FIELD = Code.PREFIX + "outlineDrawn";

    OutlineSkip() {
        super("outline-skip", 1, "no full-screen outline pass when nothing glows (Minecraft 1.20 - 1.21.1)",
                new String[][] {{"shaders/post/entity_outline.json", "final"}}, new String[] {"optifine", "vulkanmod"},
                "net/minecraft/client/renderer/LevelRenderer", "net/minecraft/class_761");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        // 1. this.entityTarget = this.entityEffect.getTempTarget("final")
        String chain = null;
        String effectField = null;
        String targetField = null;
        String targetType = null;
        int finals = 0;
        for (MethodNode m : t.stringUsers("final")) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof LdcInsnNode) || !"final".equals(((LdcInsnNode) n).cst)) {
                    continue;
                }
                AbstractInsnNode get = Code.prev(n);
                AbstractInsnNode call = Code.next(n);
                AbstractInsnNode put = call == null ? null : Code.next(call);
                if (!(get instanceof FieldInsnNode) || get.getOpcode() != Opcodes.GETFIELD
                        || !(call instanceof MethodInsnNode) || call.getOpcode() != Opcodes.INVOKEVIRTUAL
                        || !(put instanceof FieldInsnNode) || put.getOpcode() != Opcodes.PUTFIELD) {
                    continue;
                }
                FieldInsnNode g = (FieldInsnNode) get;
                MethodInsnNode c = (MethodInsnNode) call;
                FieldInsnNode p = (FieldInsnNode) put;
                Type ret = Type.getReturnType(c.desc);
                if (!g.owner.equals(t.name) || !p.owner.equals(t.name) || !c.desc.startsWith("(Ljava/lang/String;)L")
                        || !g.desc.equals("L" + c.owner + ";") || !p.desc.equals(ret.getDescriptor())) {
                    continue;
                }
                finals++;
                chain = c.owner;
                effectField = g.name;
                targetField = p.name;
                targetType = ret.getInternalName();
            }
        }
        if (finals == 0) {
            throw Skip.notTarget("no outline target here");
        }
        if (finals != 1) {
            throw Skip.because("the outline target is set up in " + finals + " places");
        }

        // 2. if (flag) { this.entityEffect.process(x); ... } - the only (F)V call on the outline effect
        AbstractInsnNode flagLoad = null;
        MethodNode render = null;
        int processCalls = 0;
        for (Object[] site : t.calls(Opcodes.INVOKEVIRTUAL, chain, null, "(F)V")) {
            MethodNode m = (MethodNode) site[0];
            AbstractInsnNode call = (AbstractInsnNode) site[1];
            AbstractInsnNode get = null;
            AbstractInsnNode n = call;
            for (int i = 0; i < 5 && n != null; i++) {
                n = Code.prev(n);
                if (n instanceof FieldInsnNode && n.getOpcode() == Opcodes.GETFIELD) {
                    get = n;
                    break;
                }
            }
            if (get == null || !((FieldInsnNode) get).name.equals(effectField)
                    || !((FieldInsnNode) get).owner.equals(t.name)) {
                continue;
            }
            processCalls++;
            AbstractInsnNode self = Code.prev(get);
            AbstractInsnNode jump = self == null ? null : Code.prev(self);
            AbstractInsnNode load = jump == null ? null : Code.prev(jump);
            if (Code.isVar(self, Opcodes.ALOAD, 0) && jump instanceof JumpInsnNode && jump.getOpcode() == Opcodes.IFEQ
                    && load instanceof VarInsnNode && load.getOpcode() == Opcodes.ILOAD) {
                flagLoad = load;
                render = m;
            }
        }
        if (processCalls != 1 || flagLoad == null) {
            throw Skip.because("the outline effect is used in an unexpected way (" + processCalls + " calls)");
        }

        // 3. doEntityOutline: if (this.shouldShow()) { ... this.entityTarget.blit(w, h, false) ... }
        MethodNode blitMethod = null;
        AbstractInsnNode showCall = null;
        for (Object[] site : t.calls(Opcodes.INVOKEVIRTUAL, targetType, null, "(IIZ)V")) {
            MethodNode m = (MethodNode) site[0];
            if (!m.desc.equals("()V") || (m.access & Opcodes.ACC_STATIC) != 0 || !readsField(m, t.name, targetField)) {
                continue;
            }
            AbstractInsnNode self = Code.first(m);
            AbstractInsnNode show = self == null ? null : Code.next(self);
            AbstractInsnNode jump = show == null ? null : Code.next(show);
            if (Code.isVar(self, Opcodes.ALOAD, 0) && show instanceof MethodInsnNode
                    && ((MethodInsnNode) show).owner.equals(t.name) && ((MethodInsnNode) show).desc.equals("()Z")
                    && jump != null && jump.getOpcode() == Opcodes.IFEQ) {
                if (blitMethod != null && blitMethod != m) {
                    throw Skip.because("more than one method draws the outline layer");
                }
                blitMethod = m;
                showCall = show;
            }
        }
        if (blitMethod == null) {
            throw Skip.because("the outline layer is not drawn the expected way");
        }
        for (MethodNode m : new MethodNode[] {render, blitMethod}) {
            if (Code.changedByMixin(m) || Code.callsModLoader(m)) {
                throw Skip.because("the outline code is changed or hooked by a mod or mod loader");
            }
        }

        t.addField(new FieldNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC, FIELD, "Z", null,
                null));
        MethodNode helper = helper(t.name, t.needsFrames());
        t.add(helper);
        InsnList store = new InsnList();
        store.add(new VarInsnNode(Opcodes.ILOAD, ((VarInsnNode) flagLoad).var));
        store.add(new FieldInsnNode(Opcodes.PUTSTATIC, t.name, FIELD, "Z"));
        render.instructions.insertBefore(flagLoad, store);
        render.maxStack = Math.max(render.maxStack, 1);
        blitMethod.instructions.insert(showCall,
                new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, helper.name, helper.desc, false));
        t.changed(render);
        t.changed(blitMethod);
    }

    /** static boolean blockforge$lite$outline(boolean show) { return show &amp;&amp; outlineDrawn; } */
    static MethodNode helper(String owner, boolean frames) {
        MethodNode h = new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC,
                Code.PREFIX + "outline", "(Z)Z", null, null);
        Label no = new Label();
        h.visitCode();
        h.visitVarInsn(Opcodes.ILOAD, 0);
        h.visitJumpInsn(Opcodes.IFEQ, no);
        h.visitFieldInsn(Opcodes.GETSTATIC, owner, FIELD, "Z");
        h.visitJumpInsn(Opcodes.IFEQ, no);
        h.visitInsn(Opcodes.ICONST_1);
        h.visitInsn(Opcodes.IRETURN);
        h.visitLabel(no);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        }
        h.visitInsn(Opcodes.ICONST_0);
        h.visitInsn(Opcodes.IRETURN);
        h.visitMaxs(1, 1);
        h.visitEnd();
        return h;
    }

    private static boolean readsField(MethodNode m, String owner, String field) {
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            if (n instanceof FieldInsnNode && n.getOpcode() == Opcodes.GETFIELD && ((FieldInsnNode) n).owner.equals(owner)
                    && ((FieldInsnNode) n).name.equals(field)) {
                return true;
            }
        }
        return false;
    }
}
