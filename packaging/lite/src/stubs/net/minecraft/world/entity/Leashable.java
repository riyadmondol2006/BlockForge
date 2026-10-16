package net.minecraft.world.entity;

/** Stand-in of the game's interface (see {@link net.minecraft.client.Minecraft}). */
public interface Leashable {
    default Entity getLeashHolder() {
        return null;
    }
}
