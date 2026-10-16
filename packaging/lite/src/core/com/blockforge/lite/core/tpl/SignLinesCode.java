package com.blockforge.lite.core.tpl;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.util.FormattedCharSequence;

/**
 * Template of the code sign-lines adds to AbstractSignRenderer (Minecraft 26.x names). Never loaded: {@code Template}
 * copies it into the game class; see {@code SignLines}.
 */
final class SignLinesCode {
    private SignLinesCode() {}

    /** collector.submitText(...), except for an empty line without a background, which would draw nothing. */
    static void blockforge$lite$signLine(SubmitNodeCollector collector, PoseStack pose, float x, float y,
            FormattedCharSequence line, boolean dropShadow, Font.DisplayMode mode, int light, int color, int background,
            int outline) {
        if (line == FormattedCharSequence.EMPTY && background == 0) {
            return;
        }
        collector.submitText(pose, x, y, line, dropShadow, mode, light, color, background, outline);
    }
}
