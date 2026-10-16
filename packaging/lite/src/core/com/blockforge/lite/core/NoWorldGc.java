package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.Type;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * no-world-gc: Minecraft 1.7 - 1.12 asks Java for a full garbage collection (System.gc()) every time a world is
 * opened or left and on every trip through a portal. On Java 8 that is one long pause of the whole game (seconds with a
 * big modpack) on top of the loading screen; Java collects the same memory by itself a little later, in the
 * background. The patch removes exactly those two calls:
 * <pre>
 *   launchIntegratedServer(...):  this.loadWorld(null); System.gc(); ...
 *   loadWorld(world, text):       ... System.gc(); this.systemTime = 0L; return;
 * </pre>
 * The other System.gc() calls (when memory runs out, when the game closes) stay, and so does Java's own use of it
 * for native buffers, which the JVM switch -XX:+DisableExplicitGC would also turn off. Newer versions only call it
 * when memory runs out, so they don't match.
 */
final class NoWorldGc extends Patch {
    NoWorldGc() {
        super("no-world-gc", 1, "no full garbage collection pause when a world opens or closes (Minecraft 1.7 - 1.12)",
                new String[][] {{"java/lang/System", "gc", "fpslimit_wait"}}, new String[0], IdleFps.MINECRAFT);
    }

    @Override
    void apply(Target c, Ctx ctx) throws Skip {
        List<Object[]> calls = c.calls(Opcodes.INVOKESTATIC, "java/lang/System", "gc", "()V");
        if (calls.isEmpty()) {
            throw Skip.notTarget("no System.gc()");
        }
        List<MethodNode> changed = new ArrayList<MethodNode>();
        List<AbstractInsnNode> remove = new ArrayList<AbstractInsnNode>();
        for (Object[] site : calls) {
            MethodNode m = (MethodNode) site[0];
            AbstractInsnNode gc = (AbstractInsnNode) site[1];
            if (afterUnload(c, m, gc) || endOfLoadWorld(c, m, gc)) {
                remove.add(gc);
                if (!changed.contains(m)) {
                    changed.add(m);
                }
            }
        }
        if (remove.isEmpty()) {
            throw Skip.because("System.gc() is not where it is expected (newer Minecraft, or changed by a mod)");
        }
        if (remove.size() > 2) {
            throw Skip.because("more places than expected");
        }
        for (Object[] site : calls) {
            if (remove.contains(site[1])) {
                ((MethodNode) site[0]).instructions.remove((AbstractInsnNode) site[1]);
            }
        }
        for (MethodNode m : changed) {
            c.changed(m);
        }
    }

    /**
     * launchIntegratedServer: this.loadWorld(null); System.gc(); - first in vanilla, after Forge's own call in Forge
     * (where the null is also cast: aconst_null; checkcast WorldClient). The same lines in freeMemory (out of memory)
     * are inside a try block, so their System.gc() follows the catch, not the call, and stays.
     */
    private static boolean afterUnload(Target c, MethodNode m, AbstractInsnNode gc) {
        AbstractInsnNode load = Code.prev(gc);
        AbstractInsnNode nul = load == null ? null : Code.prev(load);
        if (nul != null && nul.getOpcode() == Opcodes.CHECKCAST) {
            nul = Code.prev(nul);
        }
        AbstractInsnNode self = nul == null ? null : Code.prev(nul);
        if (self == null || !Code.isVar(self, Opcodes.ALOAD, 0) || nul.getOpcode() != Opcodes.ACONST_NULL
                || !(load instanceof MethodInsnNode)) {
            return false;
        }
        MethodInsnNode l = (MethodInsnNode) load;
        Type[] args = Type.getArgumentTypes(l.desc);
        return l.getOpcode() == Opcodes.INVOKEVIRTUAL && l.owner.equals(c.name) && args.length == 1
                && args[0].getSort() == Type.OBJECT && Type.getReturnType(l.desc) == Type.VOID_TYPE
                && (m.access & Opcodes.ACC_STATIC) == 0;
    }

    /** The end of loadWorld(world, text): System.gc(); this.systemTime = 0L; return; */
    private static boolean endOfLoadWorld(Target c, MethodNode m, AbstractInsnNode gc) {
        Type[] args = Type.getArgumentTypes(m.desc);
        if ((m.access & Opcodes.ACC_STATIC) != 0 || args.length != 2 || args[0].getSort() != Type.OBJECT
                || !args[1].getDescriptor().equals("Ljava/lang/String;")
                || Type.getReturnType(m.desc) != Type.VOID_TYPE) {
            return false;
        }
        AbstractInsnNode self = Code.next(gc);
        AbstractInsnNode zero = self == null ? null : Code.next(self);
        AbstractInsnNode put = zero == null ? null : Code.next(zero);
        AbstractInsnNode ret = put == null ? null : Code.next(put);
        return Code.isVar(self, Opcodes.ALOAD, 0) && zero.getOpcode() == Opcodes.LCONST_0
                && put instanceof FieldInsnNode && put.getOpcode() == Opcodes.PUTFIELD
                && ((FieldInsnNode) put).owner.equals(c.name) && ((FieldInsnNode) put).desc.equals("J")
                && ret != null && ret.getOpcode() == Opcodes.RETURN && Code.next(ret) == null;
    }
}
