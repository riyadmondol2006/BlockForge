package com.blockforge.lite.core;

import java.util.ArrayList;
import java.util.List;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * short-fade: when the game has finished loading (at the start and after a resource reload), the Mojang loading
 * screen still stays for 2 seconds while it fades out, and keys and clicks are ignored all that time. After an
 * in-game reload it is also held for at least another second while it fades in. This makes both fades four times
 * shorter (0.5 s and 0.25 s). Loading itself is not changed; only the animation after it.
 *
 * Target, the same in 1.20 - 26.x: in LoadingOverlay's render method
 * <pre>
 *   float fadeOut = ... (now - fadeOutStart) / 1000.0F ...;   float fadeIn = ... (now - fadeInStart) / 500.0F ...;
 * </pre>
 * and, from 1.21.2 on, in isReadyToFadeOut(): {@code now - fadeInStart >= 1000L}. The constants are replaced (the same
 * kind of constant, so nothing else in the code changes).
 */
final class ShortFade extends Patch {
    static final float FADE_OUT = 250.0F;
    static final float FADE_IN = 125.0F;
    static final long READY = 250L;

    ShortFade() {
        super("short-fade", 1, "the loading screen goes away 1.5 s sooner after loading (shorter fade)",
                new String[][] {{"textures/gui/title/mojangstudios.png"}}, new String[0],
                "net/minecraft/client/gui/screens/LoadingOverlay", "net/minecraft/class_425");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<MethodNode> methods = t.allMethods();
        if (methods.size() > 40) {
            throw Skip.notTarget("too big for the loading overlay");
        }
        MethodNode render = null;
        AbstractInsnNode out = null;
        AbstractInsnNode in = null;
        List<AbstractInsnNode> ready = new ArrayList<AbstractInsnNode>();
        MethodNode readyMethod = null;
        for (MethodNode m : methods) {
            if (m.instructions == null) {
                continue;
            }
            AbstractInsnNode o = null;
            AbstractInsnNode i = null;
            int outs = 0;
            int ins = 0;
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (isDivisor(n, 1000.0F)) {
                    o = n;
                    outs++;
                } else if (isDivisor(n, 500.0F)) {
                    i = n;
                    ins++;
                } else if (m.desc.equals("()Z") && n instanceof LdcInsnNode
                        && Long.valueOf(1000L).equals(((LdcInsnNode) n).cst) && Code.next(n) != null
                        && Code.next(n).getOpcode() == Opcodes.LCMP) {
                    ready.add(n);
                    readyMethod = m;
                }
            }
            if (outs + ins == 0) {
                continue;
            }
            if (outs != 1 || ins != 1 || render != null || (m.access & Opcodes.ACC_STATIC) != 0
                    || !m.desc.endsWith("IIF)V")) {
                throw Skip.because("the fade times are not where they are expected");
            }
            render = m;
            out = o;
            in = i;
        }
        if (render == null) {
            throw Skip.notTarget("no fade times here");
        }
        if (Code.changedByMixin(render) || (readyMethod != null && Code.changedByMixin(readyMethod))) {
            throw Skip.because("the loading overlay was changed by a mod");
        }
        if (ready.size() > 1) {
            throw Skip.because("isReadyToFadeOut looks different");
        }
        ((LdcInsnNode) out).cst = FADE_OUT;
        ((LdcInsnNode) in).cst = FADE_IN;
        t.changed(render);
        if (readyMethod != null) {
            ((LdcInsnNode) ready.get(0)).cst = READY;
            t.changed(readyMethod);
        }
    }

    /** ldc value; fdiv - after a long-to-float conversion, as in "(float)(now - start) / value". */
    private static boolean isDivisor(AbstractInsnNode n, float value) {
        if (!(n instanceof LdcInsnNode) || !Float.valueOf(value).equals(((LdcInsnNode) n).cst)) {
            return false;
        }
        AbstractInsnNode prev = Code.prev(n);
        AbstractInsnNode next = Code.next(n);
        return prev != null && prev.getOpcode() == Opcodes.L2F && next != null && next.getOpcode() == Opcodes.FDIV;
    }
}
