package net.minecraft.client.renderer;

import net.minecraft.client.Camera;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class GameRenderer {
    public Camera camera = new Camera();

    public Camera mainCamera() {
        return camera;
    }
}
