package net.minecraft.core;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}), with the game's packing. */
public class SectionPos {
    public static long asLong(int x, int y, int z) {
        return ((long) x & 0x3FFFFFL) << 42 | (long) y & 0xFFFFFL | ((long) z & 0x3FFFFFL) << 20;
    }

    public static int x(long node) {
        return (int) (node << 0 >> 42);
    }

    public static int y(long node) {
        return (int) (node << 44 >> 44);
    }

    public static int z(long node) {
        return (int) (node << 22 >> 42);
    }
}
