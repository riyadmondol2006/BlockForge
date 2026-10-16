package com.blockforge.lite.core;

import java.util.List;
import org.objectweb.asm.Label;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.JumpInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;
import org.objectweb.asm.tree.VarInsnNode;

/**
 * idle-fps: while the game window is minimised (or is a full screen the player switched away from), draw 10 frames a
 * second instead of the full frame rate. Minecraft 1.21.2 and newer do exactly this themselves; this brings it to the
 * older versions, where a minimised game keeps the processor and graphics chip as busy as a visible one.
 *
 * Targets, the end of Minecraft's frame:
 * <pre>
 *   1.14 - 1.21.1 (LWJGL 3):   int limit = this.getFramerateLimit();
 *                              if (limit < 260) RenderSystem.limitDisplayFPS(limit);
 *   1.8 - 1.12 (LWJGL 2):      if (this.isFramerateLimitBelowMax()) Display.sync(this.getLimitFramerate());
 * </pre>
 * The patch passes the value of getFramerateLimit() through a small method it adds to the same class, which asks the
 * window and returns 10 or the value unchanged. Nothing else changes; the added method calls only GLFW (or LWJGL 2's
 * Display) and the JDK, so it works with every mod loader. On 1.21.2 and newer the limit comes from Minecraft's own
 * tracker instead, so the shape does not match and the class is left alone.
 */
final class IdleFps extends Patch {
    static final String RENDER_SYSTEM = "com/mojang/blaze3d/systems/RenderSystem";
    static final String GLFW = "org/lwjgl/glfw/GLFW";
    static final String DISPLAY = "org/lwjgl/opengl/Display";
    static final int GLFW_FOCUSED = 0x20001;
    static final int GLFW_ICONIFIED = 0x20002;
    /** What Minecraft 1.21.2+ uses for a minimised window. */
    static final int IDLE_LIMIT = 10;

    IdleFps() {
        super("idle-fps", 1, "10 frames a second while the window is minimised, like Minecraft 1.21.2 and newer",
                new String[][] {{RENDER_SYSTEM, "limitDisplayFPS"}, {DISPLAY, "sync"}},
                new String[] {"dynamicfps", "fpsreducer", "vulkanmod"}, MINECRAFT);
    }

    /** The Minecraft class: Mojang's name (Forge, NeoForge, new versions), Fabric's intermediary name. */
    static final String[] MINECRAFT = {"net/minecraft/client/Minecraft", "net/minecraft/class_310"};

    @Override
    void apply(Target c, Ctx ctx) throws Skip {
        List<Object[]> modern = c.calls(Opcodes.INVOKESTATIC, RENDER_SYSTEM, "limitDisplayFPS", "(I)V");
        List<Object[]> legacy = c.calls(Opcodes.INVOKESTATIC, DISPLAY, "sync", "(I)V");
        if (modern.isEmpty() && legacy.isEmpty()) {
            throw Skip.notTarget("no frame limiter call");
        }
        if (modern.size() + legacy.size() != 1) {
            throw Skip.because("the frame limiter is called " + (modern.size() + legacy.size()) + " times");
        }
        if (c.isInterface()) {
            throw Skip.because("the caller is an interface");
        }
        boolean lwjgl3 = !modern.isEmpty();
        Object[] site = lwjgl3 ? modern.get(0) : legacy.get(0);
        MethodNode m = (MethodNode) site[0];
        AbstractInsnNode call = (AbstractInsnNode) site[1];
        AbstractInsnNode get = lwjgl3 ? modernSource(m, call) : Code.prev(call);
        AbstractInsnNode self = get == null ? null : Code.prev(get);
        if (!(get instanceof MethodInsnNode) || (get.getOpcode() != Opcodes.INVOKEVIRTUAL
                && get.getOpcode() != Opcodes.INVOKESPECIAL) || !Code.isVar(self, Opcodes.ALOAD, 0)
                || (m.access & Opcodes.ACC_STATIC) != 0) {
            throw Skip.because("the frame limit does not come from a method of this class");
        }
        MethodInsnNode limit = (MethodInsnNode) get;
        if (!limit.owner.equals(c.name) || !limit.desc.equals("()I") || Code.isAddedName(limit.name)) {
            throw Skip.because("the frame limit comes from " + limit.owner + "." + limit.name + limit.desc);
        }
        MethodNode source = c.method(limit.name, limit.desc);
        // in menus outside a world the limit is 60 (LWJGL 3 versions) or 30 (LWJGL 2 versions)
        if (source == null || (source.access & Opcodes.ACC_STATIC) != 0 || !menuLimit(source, lwjgl3 ? 60 : 30)) {
            throw Skip.because("getFramerateLimit looks different");
        }
        if (Code.changedByMixin(source)) {
            throw Skip.because("getFramerateLimit was changed by a mod");
        }

        MethodNode helper = lwjgl3 ? glfwHelper(c.needsFrames()) : displayHelper(c.needsFrames());
        c.add(helper);
        m.instructions.insert(get, new MethodInsnNode(Opcodes.INVOKESTATIC, c.name, helper.name, helper.desc, false));
        c.changed(m);
    }

    /** ILOAD n <- IF_ICMPGE <- SIPUSH 260 <- ILOAD n <- ISTORE n <- (the call that gives the limit); or null. */
    private static AbstractInsnNode modernSource(MethodNode m, AbstractInsnNode call) throws Skip {
        AbstractInsnNode load2 = Code.prev(call);
        AbstractInsnNode jump = load2 == null ? null : Code.prev(load2);
        AbstractInsnNode max = jump == null ? null : Code.prev(jump);
        AbstractInsnNode load1 = max == null ? null : Code.prev(max);
        AbstractInsnNode store = load1 == null ? null : Code.prev(load1);
        if (!(load2 instanceof VarInsnNode) || load2.getOpcode() != Opcodes.ILOAD) {
            throw Skip.because("unexpected code before limitDisplayFPS");
        }
        int var = ((VarInsnNode) load2).var;
        if (!(jump instanceof JumpInsnNode) || jump.getOpcode() != Opcodes.IF_ICMPGE || !Code.isInt(max, 260)
                || !Code.isVar(load1, Opcodes.ILOAD, var) || !Code.isVar(store, Opcodes.ISTORE, var)) {
            throw Skip.because("the frame limit is not read the expected way (newer Minecraft, or changed by a mod)");
        }
        return Code.prev(store);
    }

    /** getFramerateLimit(): short, returns a fixed limit in menus outside a world. */
    private static boolean menuLimit(MethodNode m, int menu) {
        if (m.instructions == null || m.instructions.size() > 60) {
            return false;
        }
        boolean fixed = false;
        boolean ret = false;
        for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
            fixed |= Code.isInt(n, menu);
            ret |= n.getOpcode() == Opcodes.IRETURN;
        }
        return fixed && ret;
    }

    /**
     * <pre>
     * private static int blockforge$lite$idleFps(int limit) {
     *     try {
     *         long w = GLFW.glfwGetCurrentContext();
     *         if (w != 0 && (GLFW.glfwGetWindowAttrib(w, GLFW_ICONIFIED) != 0
     *                 || GLFW.glfwGetWindowMonitor(w) != 0 && GLFW.glfwGetWindowAttrib(w, GLFW_FOCUSED) == 0)) {
     *             return Math.min(limit, 10);
     *         }
     *     } catch (Throwable e) {
     *     }
     *     return limit;
     * }
     * </pre>
     * The window's GL context belongs to the render thread, which is the thread that calls this.
     */
    static MethodNode glfwHelper(boolean frames) {
        MethodNode h = helperNode();
        Label start = new Label();
        Label end = new Label();
        Label idle = new Label();
        Label keep = new Label();
        Label handler = new Label();
        h.visitCode();
        h.visitTryCatchBlock(start, end, handler, "java/lang/Throwable");
        h.visitLabel(start);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, GLFW, "glfwGetCurrentContext", "()J", false);
        h.visitVarInsn(Opcodes.LSTORE, 1);
        h.visitVarInsn(Opcodes.LLOAD, 1);
        h.visitInsn(Opcodes.LCONST_0);
        h.visitInsn(Opcodes.LCMP);
        h.visitJumpInsn(Opcodes.IFEQ, keep);
        h.visitVarInsn(Opcodes.LLOAD, 1);
        h.visitLdcInsn(GLFW_ICONIFIED);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, GLFW, "glfwGetWindowAttrib", "(JI)I", false);
        h.visitJumpInsn(Opcodes.IFNE, idle);
        h.visitVarInsn(Opcodes.LLOAD, 1);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, GLFW, "glfwGetWindowMonitor", "(J)J", false);
        h.visitInsn(Opcodes.LCONST_0);
        h.visitInsn(Opcodes.LCMP);
        h.visitJumpInsn(Opcodes.IFEQ, keep);
        h.visitVarInsn(Opcodes.LLOAD, 1);
        h.visitLdcInsn(GLFW_FOCUSED);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, GLFW, "glfwGetWindowAttrib", "(JI)I", false);
        h.visitJumpInsn(Opcodes.IFNE, keep);
        h.visitLabel(idle);
        if (frames) {
            h.visitFrame(Opcodes.F_FULL, 2, new Object[] {Opcodes.INTEGER, Opcodes.LONG}, 0, new Object[0]);
        }
        returnIdle(h);
        h.visitLabel(keep);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        }
        h.visitVarInsn(Opcodes.ILOAD, 0);
        h.visitInsn(Opcodes.IRETURN);
        h.visitLabel(end);
        handler(h, handler, frames);
        h.visitMaxs(4, 3);
        h.visitEnd();
        return h;
    }

    /**
     * The same for LWJGL 2:
     * <pre>
     * if (!Display.isVisible() || Display.isFullscreen() && !Display.isActive()) return Math.min(limit, 10);
     * </pre>
     */
    static MethodNode displayHelper(boolean frames) {
        MethodNode h = helperNode();
        Label start = new Label();
        Label end = new Label();
        Label idle = new Label();
        Label keep = new Label();
        Label handler = new Label();
        h.visitCode();
        h.visitTryCatchBlock(start, end, handler, "java/lang/Throwable");
        h.visitLabel(start);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, DISPLAY, "isVisible", "()Z", false);
        h.visitJumpInsn(Opcodes.IFEQ, idle);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, DISPLAY, "isFullscreen", "()Z", false);
        h.visitJumpInsn(Opcodes.IFEQ, keep);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, DISPLAY, "isActive", "()Z", false);
        h.visitJumpInsn(Opcodes.IFNE, keep);
        h.visitLabel(idle);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        }
        returnIdle(h);
        h.visitLabel(keep);
        if (frames) {
            h.visitFrame(Opcodes.F_SAME, 0, null, 0, null);
        }
        h.visitVarInsn(Opcodes.ILOAD, 0);
        h.visitInsn(Opcodes.IRETURN);
        h.visitLabel(end);
        handler(h, handler, frames);
        h.visitMaxs(2, 1);
        h.visitEnd();
        return h;
    }

    private static MethodNode helperNode() {
        return new MethodNode(Opcodes.ACC_PRIVATE | Opcodes.ACC_STATIC | Opcodes.ACC_SYNTHETIC,
                Code.PREFIX + "idleFps", "(I)I", null, null);
    }

    private static void returnIdle(MethodNode h) {
        h.visitVarInsn(Opcodes.ILOAD, 0);
        h.visitIntInsn(Opcodes.BIPUSH, IDLE_LIMIT);
        h.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Math", "min", "(II)I", false);
        h.visitInsn(Opcodes.IRETURN);
    }

    /** catch (Throwable e) { return limit; } */
    private static void handler(MethodNode h, Label handler, boolean frames) {
        h.visitLabel(handler);
        if (frames) {
            h.visitFrame(Opcodes.F_FULL, 1, new Object[] {Opcodes.INTEGER}, 1, new Object[] {"java/lang/Throwable"});
        }
        h.visitInsn(Opcodes.POP);
        h.visitVarInsn(Opcodes.ILOAD, 0);
        h.visitInsn(Opcodes.IRETURN);
    }
}
