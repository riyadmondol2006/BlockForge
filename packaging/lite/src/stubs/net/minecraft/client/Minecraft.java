package net.minecraft.client;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.world.entity.Entity;

/**
 * Stand-in of the game's class (Minecraft 26.x names). The patch templates (core/.../tpl) are compiled against these
 * stand-ins, and the self-test runs the templates on them. Each has only the members the templates use, with the
 * game's exact signatures (static or not, class or interface); a template's use of a member is checked against the
 * running game before it is copied, so a stand-in can never make a patch call something that is not there. The
 * fields without a counterpart in the game are for the self-test only. Never loaded inside a game.
 */
public class Minecraft {
    public static Minecraft instance;
    public ClientLevel level;
    public LevelRenderer levelRenderer = new LevelRenderer();
    public GameRenderer gameRenderer = new GameRenderer();
    public Entity cameraEntity;

    public static Minecraft getInstance() {
        return instance;
    }

    public Entity getCameraEntity() {
        return cameraEntity;
    }

    public boolean shouldEntityAppearGlowing(Entity entity) {
        return entity.glowing;
    }
}
