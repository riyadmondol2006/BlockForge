package com.blockforge.lite.core.tpl;

import java.util.Arrays;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.Map;
import net.minecraft.client.Minecraft;
import net.minecraft.core.Vec3i;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.phys.AABB;

/**
 * Template of the code block-entity-view adds to BlockEntityRenderDispatcher (Minecraft 26.2+ names). Never loaded:
 * {@code Template} copies these members into the game class; see {@code BlockEntityView}.
 */
final class BlockEntityViewCode {
    private BlockEntityViewCode() {}

    /** Renderer class -> whether it draws only within a block of its position (copied on change, never changed). */
    private static volatile Map<Class<?>, Boolean> blockforge$lite$compact;

    /**
     * Called right after the renderer said the block entity is near enough to draw: whether it is in the field of view
     * too. Only for the game's own renderers whose drawing stays within a block of the position; for everything
     * else, and on any error, the answer is the renderer's.
     */
    static boolean blockforge$lite$beInView(boolean near, BlockEntity entity, Object renderer) {
        if (!near) {
            return false;
        }
        try {
            if (!blockforge$lite$compact(renderer.getClass())) {
                return true;
            }
            Minecraft mc = Minecraft.getInstance();
            if (mc.level == null) {
                return true;
            }
            Vec3i p = entity.getBlockPos();
            // one block around it, two above (banners, items on top)
            return mc.gameRenderer.mainCamera().getCullFrustum().isVisible(new AABB(p.getX() - 1.0, p.getY() - 1.0,
                    p.getZ() - 1.0, p.getX() + 2.0, p.getY() + 3.0, p.getZ() + 2.0));
        } catch (Throwable e) {
            return true;
        }
    }

    private static boolean blockforge$lite$compact(Class<?> c) {
        Map<Class<?>, Boolean> known = blockforge$lite$compact;
        Boolean b = known == null ? null : known.get(c);
        if (b != null) {
            return b;
        }
        HashSet<String> names = new HashSet<String>(Arrays.asList(
                "net.minecraft.client.renderer.blockentity.ChestRenderer",
                "net.minecraft.client.renderer.blockentity.ShulkerBoxRenderer",
                "net.minecraft.client.renderer.blockentity.BannerRenderer",
                "net.minecraft.client.renderer.blockentity.StandingSignRenderer",
                "net.minecraft.client.renderer.blockentity.HangingSignRenderer",
                "net.minecraft.client.renderer.blockentity.DecoratedPotRenderer",
                "net.minecraft.client.renderer.blockentity.CampfireRenderer",
                "net.minecraft.client.renderer.blockentity.LecternRenderer",
                "net.minecraft.client.renderer.blockentity.BellRenderer",
                "net.minecraft.client.renderer.blockentity.EnchantTableRenderer",
                "net.minecraft.client.renderer.blockentity.VaultRenderer",
                "net.minecraft.client.renderer.blockentity.BrushableBlockRenderer",
                "net.minecraft.client.renderer.blockentity.ShelfRenderer",
                "net.minecraft.client.renderer.blockentity.CopperGolemStatueBlockRenderer"));
        // exactly the game's class: a mod's subclass may draw more
        b = names.contains(c.getName());
        Map<Class<?>, Boolean> next = known == null ? new IdentityHashMap<Class<?>, Boolean>()
                : new IdentityHashMap<Class<?>, Boolean>(known);
        next.put(c, b);
        blockforge$lite$compact = next;
        return b;
    }
}
