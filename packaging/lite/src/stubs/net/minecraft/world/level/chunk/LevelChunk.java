package net.minecraft.world.level.chunk;

import net.minecraft.world.level.Level;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class LevelChunk extends ChunkAccess {
    private final Level level;
    private final int x;
    private final int z;

    public LevelChunk(Level level, int x, int z) {
        this.level = level;
        this.x = x;
        this.z = z;
    }

    @Override
    public LevelChunkSection getSection(int index) {
        if (index < 0 || index >= 24) {
            throw new ArrayIndexOutOfBoundsException(index);
        }
        return new LevelChunkSection(level.air.contains(x + "," + (index - 4) + "," + z));
    }
}
