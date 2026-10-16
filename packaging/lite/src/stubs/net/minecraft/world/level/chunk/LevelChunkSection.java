package net.minecraft.world.level.chunk;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class LevelChunkSection {
    private final boolean air;

    public LevelChunkSection(boolean air) {
        this.air = air;
    }

    public boolean hasOnlyAir() {
        return air;
    }
}
