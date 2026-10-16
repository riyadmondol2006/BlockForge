package net.minecraft.client;

import net.minecraft.client.renderer.culling.Frustum;

/** Stand-in of the game's class (see {@link Minecraft}). */
public class Camera {
    public Frustum cullFrustum = new Frustum();

    public Frustum getCullFrustum() {
        return cullFrustum;
    }
}
