package net.minecraft.client.renderer.culling;

import net.minecraft.world.phys.AABB;

/**
 * Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}): everything is in view, or nothing, or
 * what starts before x = {@link #endX}.
 */
public class Frustum {
    public boolean all = true;
    public double endX = Double.MAX_VALUE;

    public boolean isVisible(AABB box) {
        return all && !(box.minX >= endX);
    }
}
