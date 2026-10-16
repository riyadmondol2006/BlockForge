package net.minecraft.core;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class Vec3i {
    private final int x;
    private final int y;
    private final int z;

    public Vec3i(int x, int y, int z) {
        this.x = x;
        this.y = y;
        this.z = z;
    }

    public int getX() {
        return x;
    }

    public int getY() {
        return y;
    }

    public int getZ() {
        return z;
    }
}
