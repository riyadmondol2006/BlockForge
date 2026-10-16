package com.blockforge.lite.core;

import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * short-spin (level 2): Minecraft 26.1 and newer wait for the next frame in FramerateLimiter.limitDisplayFPS: they
 * sleep until "average oversleep + 0.5 ms" before the frame is due and then busy-wait (Thread.onSpinWait) for the
 * rest, so every frame keeps a processor core spinning for 0.5 - 2.5 ms (3 - 15 % of a core at 60 frames a second,
 * more at higher limits). Only used when the frame rate is limited (menus, or a limit below "unlimited").
 *
 * The patch shortens the busy part: 0.1 ms instead of 0.5 ms of safety margin, and the oversleep allowance is capped
 * at 1 ms instead of 2 ms. The waiting itself is unchanged; a frame can come up to about half a millisecond later
 * than before when the system oversleeps more than usual. Constant changes only:
 * <pre>
 *   ldc2_w 500000L (twice)          becomes  ldc2_w 100000L
 *   ldc2_w 2000000L; Math.min(JJ)J   becomes  ldc2_w 1000000L; Math.min(JJ)J
 * </pre>
 */
final class ShortSpin extends Patch {
    static final long SPIN = 500000L;
    static final long NEW_SPIN = 100000L;
    static final long CAP = 2000000L;
    static final long NEW_CAP = 1000000L;

    ShortSpin() {
        super("short-spin", 2, "the frame limiter busy-waits 0.1 ms instead of 0.5 - 2.5 ms per frame (less processor "
                + "time; frames may come up to 0.5 ms later; Minecraft 26.1 and newer)",
                new String[][] {{"java/util/concurrent/locks/LockSupport", "parkNanos", "onSpinWait"}}, new String[0],
                "net/minecraft/client/FramerateLimiter");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<Object[]> spins = t.calls(Opcodes.INVOKESTATIC, "java/lang/Thread", "onSpinWait", "()V");
        if (spins.isEmpty()) {
            throw Skip.notTarget("no Thread.onSpinWait()");
        }
        MethodNode m = (MethodNode) spins.get(0)[0];
        if (spins.size() != 1 || !m.desc.equals("(I)V") || (m.access & Opcodes.ACC_STATIC) == 0
                || Code.calls(m, Opcodes.INVOKESTATIC, "java/util/concurrent/locks/LockSupport", "parkNanos", "(J)V")
                        .size() != 1) {
            throw Skip.notTarget("not the frame limiter");
        }
        List<AbstractInsnNode> margins = Code.ldcs(m, SPIN);
        List<AbstractInsnNode> caps = Code.ldcs(m, CAP);
        if (margins.size() != 2 || caps.size() != 1
                || !Code.isCall(Code.next(caps.get(0)), Opcodes.INVOKESTATIC, "java/lang/Math", "min", "(JJ)J")) {
            throw Skip.because("the frame limiter looks different (" + margins.size() + " margins, " + caps.size()
                    + " caps)");
        }
        if (Code.changedByMixin(m)) {
            throw Skip.because("the frame limiter is changed by a mod");
        }
        for (AbstractInsnNode n : margins) {
            m.instructions.set(n, new LdcInsnNode(NEW_SPIN));
        }
        m.instructions.set(caps.get(0), new LdcInsnNode(NEW_CAP));
        t.changed(m);
    }
}
