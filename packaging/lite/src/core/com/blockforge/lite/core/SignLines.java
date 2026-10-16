package com.blockforge.lite.core;

import java.util.List;
import java.util.Map;
import org.objectweb.asm.Opcodes;
import org.objectweb.asm.tree.MethodInsnNode;
import org.objectweb.asm.tree.MethodNode;

/**
 * sign-lines: every frame, every sign within 64 blocks hands all 4 lines of both sides to the text renderer (8 text
 * submissions, each with a matrix copy and a later text layout), also the empty ones; most signs use one or two
 * lines on one side. An empty line without a background draws nothing (no glyphs, no outline, no background), so the
 * patch drops it before it is handed on. The picture is the same.
 *
 * Target, AbstractSignRenderer.submitSignText in Minecraft 26.1 and newer:
 * <pre>
 *   for (i ...) collector.submitText(poseStack, x, y, lines[i], false, POLYGON_OFFSET, light, color, 0, outline);
 * </pre>
 * The call goes through the template {@code tpl/SignLinesCode}: same arguments, same stack effect, so the code around
 * it (a branch target with its frame) stays as it is.
 */
final class SignLines extends Patch {
    static final String SIGNS = "net/minecraft/client/renderer/blockentity/AbstractSignRenderer";
    static final String COLLECTOR = "net/minecraft/client/renderer/SubmitNodeCollector";
    static final String SUBMIT = "(Lcom/mojang/blaze3d/vertex/PoseStack;FFLnet/minecraft/util/FormattedCharSequence;Z"
            + "Lnet/minecraft/client/gui/Font$DisplayMode;IIII)V";

    SignLines() {
        super("sign-lines", 1, "empty sign lines are not handed to the text renderer (Minecraft 26.x)",
                new String[][] {{COLLECTOR, "submitText"}},
                new String[] {"immediatelyfast", "moreculling", "optifine", "vulkanmod"}, SIGNS);
    }

    @Override
    void apply(Target t, Ctx ctx) throws Skip {
        List<Object[]> sites = t.calls(Opcodes.INVOKEINTERFACE, COLLECTOR, "submitText", SUBMIT);
        if (sites.isEmpty()) {
            throw Skip.notTarget("no text submission here");
        }
        if (sites.size() != 1) {
            throw Skip.because("text is submitted in " + sites.size() + " places");
        }
        MethodNode m = (MethodNode) sites.get(0)[0];
        MethodInsnNode call = (MethodInsnNode) sites.get(0)[1];
        if (Code.changedByMixin(m) || Code.callsModLoader(m)) {
            throw Skip.because("the sign text code was changed by a mod");
        }
        Map<String, MethodNode> code = Template.copy(t, ctx, "SignLinesCode");
        MethodNode line = code.get(Code.PREFIX + "signLine");
        if (line == null || !line.desc.equals("(L" + COLLECTOR + ";" + SUBMIT.substring(1))) {
            throw new IllegalStateException("template helper missing or with another descriptor");
        }
        m.instructions.set(call, new MethodInsnNode(Opcodes.INVOKESTATIC, t.name, line.name, line.desc, false));
        t.changed(m);
    }
}
