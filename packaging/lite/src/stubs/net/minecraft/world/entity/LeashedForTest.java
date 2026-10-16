package net.minecraft.world.entity;

/** For the self-test only: an entity on a leash (no such class in the game). */
public class LeashedForTest extends Entity implements Leashable {
    public Entity holder;

    @Override
    public Entity getLeashHolder() {
        return holder;
    }
}
