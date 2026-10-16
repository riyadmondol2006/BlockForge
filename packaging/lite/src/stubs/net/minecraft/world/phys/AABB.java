package net.minecraft.world.phys;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class AABB {
    public final double minX;
    public final double minY;
    public final double minZ;
    public final double maxX;
    public final double maxY;
    public final double maxZ;

    public AABB(double minX, double minY, double minZ, double maxX, double maxY, double maxZ) {
        this.minX = minX;
        this.minY = minY;
        this.minZ = minZ;
        this.maxX = maxX;
        this.maxY = maxY;
        this.maxZ = maxZ;
    }

    public AABB inflate(double d) {
        return new AABB(minX - d, minY - d, minZ - d, maxX + d, maxY + d, maxZ + d);
    }
}
