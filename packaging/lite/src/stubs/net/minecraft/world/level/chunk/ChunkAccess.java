package net.minecraft.world.level.chunk;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public abstract class ChunkAccess {
    public abstract LevelChunkSection getSection(int index);
}
