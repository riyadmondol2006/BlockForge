package net.minecraft.client.renderer;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.gui.Font;
import net.minecraft.util.FormattedCharSequence;

/**
 * Stand-in of the game's interface (see {@link net.minecraft.client.Minecraft}); in the game submitText is declared on
 * an interface it extends, the calls name SubmitNodeCollector either way.
 */
public interface SubmitNodeCollector {
    void submitText(PoseStack poseStack, float x, float y, FormattedCharSequence string, boolean dropShadow,
            Font.DisplayMode displayMode, int lightCoords, int color, int backgroundColor, int outlineColor);
}
