package com.blockforge.lite.core;

import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.AbstractInsnNode;
import org.objectweb.asm.tree.FieldInsnNode;
import org.objectweb.asm.tree.InsnNode;
import org.objectweb.asm.tree.JumpInsnNode;
import org.objectweb.asm.tree.LdcInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * quiet-music (level 2): since 1.21.2 Minecraft starts music even when its volume is zero ("silently"), so the sound
 * thread keeps reading and decoding music files that nobody hears (A2 #7). Other sounds at volume zero are skipped:
 * <pre>
 *   if (volume == 0.0F) {
 *       if (!instance.canStartSilent() &amp;&amp; source != SoundSource.MUSIC) { log "Skipped playing sound"; return NOT_STARTED; }
 *       startedSilently = true; }
 * </pre>
 * The patch compares with null instead of SoundSource.MUSIC (getstatic becomes aconst_null), so muted music is
 * skipped like any muted sound. What the player notices: after turning the music up again, music starts with the
 * next song (after the usual pause between songs) instead of the middle of the one that was playing silently.
 *
 * Applies to 1.21.2 - 26.x (checked 1.21.11 - 26.4-snapshot-4); 1.21.1 and older already skip muted music.
 */
final class QuietMusic extends Patch {
    static final String SKIPPED = "Skipped playing sound {}, volume was zero.";

    QuietMusic() {
        super("quiet-music", 2, "muted music is not played silently in the background (no decoding nobody hears; "
                + "turned up again, music starts with the next song; Minecraft 1.21.2 and newer)",
                new String[][] {{SKIPPED}}, new String[] {"soundphysics", "ambientsounds"},
                "net/minecraft/client/sounds/SoundEngine", "net/minecraft/class_1140");
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        AbstractInsnNode music = null;
        MethodNode play = null;
        for (MethodNode m : t.stringUsers(SKIPPED)) {
            for (AbstractInsnNode n = m.instructions.getFirst(); n != null; n = n.getNext()) {
                if (!(n instanceof JumpInsnNode) || n.getOpcode() != Opcodes.IF_ACMPNE) {
                    continue;
                }
                // ...canStartSilent()Z; ifne; aload source; getstatic SoundSource.MUSIC; if_acmpne SKIP
                AbstractInsnNode[] b = Code.before(n, Opcodes.GETSTATIC, Opcodes.ALOAD, Opcodes.IFNE);
                AbstractInsnNode call = b == null ? null : Code.prev(b[2]);
                if (b == null || call == null || (call.getOpcode() != Opcodes.INVOKEINTERFACE
                        && call.getOpcode() != Opcodes.INVOKEVIRTUAL) || !Code.isCall(call, call.getOpcode(), null,
                                null, "()Z")) {
                    continue;
                }
                FieldInsnNode get = (FieldInsnNode) b[0];
                if (!get.desc.equals("L" + get.owner + ";") || !skips(((JumpInsnNode) n))) {
                    continue;
                }
                if (music != null) {
                    throw Skip.because("the volume check is there twice");
                }
                music = get;
                play = m;
            }
        }
        if (music == null) {
            throw Skip.because("no sound category starts silently here (Minecraft 1.21.1 and older skip muted music)");
        }
        if (Code.changedByMixin(play)) {
            throw Skip.because("SoundEngine.play is changed by a mod");
        }
        play.instructions.set(music, new InsnNode(Opcodes.ACONST_NULL));
        t.changed(play);
    }

    /** Whether the jump goes to the "Skipped playing sound" log line (within the next few instructions). */
    private static boolean skips(JumpInsnNode j) {
        AbstractInsnNode n = j.label;
        for (int i = 0; i < 4 && n != null; i++) {
            n = Code.next(n);
            if (n instanceof LdcInsnNode && SKIPPED.equals(((LdcInsnNode) n).cst)) {
                return true;
            }
        }
        return false;
    }
}
