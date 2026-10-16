package net.minecraft.world.level.block.entity;

import net.minecraft.core.BlockPos;

/** Stand-in of the game's class (see {@link net.minecraft.client.Minecraft}). */
public class BlockEntity {
    public BlockPos pos = new BlockPos(0, 0, 0);

    public BlockPos getBlockPos() {
        return pos;
    }
}
