package net.minecraft.world.entity;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class Entity {
    public boolean passenger;
    public boolean vehicle;
    public boolean named;
    public boolean glowing;

    public boolean isPassenger() {
        return passenger;
    }

    public boolean isVehicle() {
        return vehicle;
    }

    public boolean hasCustomName() {
        return named;
    }
}
