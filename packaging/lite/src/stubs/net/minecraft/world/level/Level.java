package net.minecraft.world.level;

import net.minecraft.world.level.chunk.LevelChunk;

/**
 * Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}): every section has blocks except the ones
 * listed in {@link #air} ("x,y,z" in section coordinates).
 */
public class Level implements LevelHeightAccessor {
    public java.util.Set<String> air = new java.util.HashSet<String>();

    public LevelChunk getChunk(int x, int z) {
        return new LevelChunk(this, x, z);
    }
}
