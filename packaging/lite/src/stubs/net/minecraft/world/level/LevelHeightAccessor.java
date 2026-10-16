package net.minecraft.world.level;

/** Stand-in of the game's interface (see {@link net.minecraft.client.Minecraft}): 24 sections from section y -4. */
public interface LevelHeightAccessor {
    default boolean isOutsideBuildHeight(int y) {
        return y < -64 || y >= 320;
    }

    default int getSectionIndexFromSectionY(int sectionY) {
        return sectionY + 4;
    }
}
