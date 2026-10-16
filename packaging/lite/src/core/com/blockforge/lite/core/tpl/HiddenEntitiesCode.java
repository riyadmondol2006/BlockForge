package com.blockforge.lite.core.tpl;

import java.util.List;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.core.SectionPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.Leashable;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.phys.AABB;

/**
 * Template of the code hidden-entities adds to EntityRenderer (Minecraft 26.2+ names). Never loaded: {@code Template}
 * copies these members into the game class; see {@code HiddenEntities}.
 */
final class HiddenEntitiesCode {
    private HiddenEntitiesCode() {}

    /**
     * What the game knew about the sections in view when it last asked: {list, its sections (a copy), bits of the
     * section grid, {x0, y0, z0, w, h, d}, the frustum of that frame}. Replaced, never changed.
     */
    private static Object[] blockforge$lite$view;

    /**
     * Called right after the game's frustum test of an entity's culling box: whether it may be seen. The box was in
     * the frustum; it is still not seen when every chunk section it touches is one the game's visibility search did
     * not reach (behind terrain), the same test the game uses for terrain. Anything unusual: seen.
     */
    static boolean blockforge$lite$inView(boolean inFrustum, Entity entity, Frustum frustum, AABB box) {
        if (!inFrustum) {
            return false;
        }
        try {
            return blockforge$lite$seen(entity, frustum, box);
        } catch (Throwable e) {
            return true;
        }
    }

    private static boolean blockforge$lite$seen(Entity entity, Frustum frustum, AABB box) {
        Minecraft mc = Minecraft.getInstance();
        ClientLevel level = mc.level;
        // only the main view of this frame (not a second camera of a mod, not a debug frustum)
        if (level == null || frustum != mc.gameRenderer.mainCamera().getCullFrustum()) {
            return true;
        }
        // these show through walls (name tags, outlines) or hang on others (riders, leads)
        if (entity instanceof Player || entity == mc.getCameraEntity() || entity.isPassenger() || entity.isVehicle()
                || entity.hasCustomName() || mc.shouldEntityAppearGlowing(entity)
                || entity instanceof Leashable && ((Leashable) entity).getLeashHolder() != null) {
            return true;
        }
        // the box half a block wider; large boxes keep the frustum test only
        double minX = box.minX - 0.5;
        double minY = box.minY - 0.5;
        double minZ = box.minZ - 0.5;
        double maxX = box.maxX + 0.5;
        double maxY = box.maxY + 0.5;
        double maxZ = box.maxZ + 0.5;
        if (!(maxX - minX <= 40.0 && maxY - minY <= 40.0 && maxZ - minZ <= 40.0)) {
            return true;
        }
        int x0 = (int) Math.floor(minX) >> 4;
        int y0 = (int) Math.floor(minY) >> 4;
        int z0 = (int) Math.floor(minZ) >> 4;
        int x1 = (int) Math.floor(maxX) >> 4;
        int y1 = (int) Math.floor(maxY) >> 4;
        int z1 = (int) Math.floor(maxZ) >> 4;
        Object[] view = blockforge$lite$view(mc.levelRenderer.visibleSections(), frustum);
        if (view == null) {
            return true;
        }
        long[] bits = (long[]) view[2];
        int[] g = (int[]) view[3];
        for (int x = x0; x <= x1; x++) {
            for (int y = y0; y <= y1; y++) {
                for (int z = z0; z <= z1; z++) {
                    int gx = x - g[0];
                    int gy = y - g[1];
                    int gz = z - g[2];
                    if (gx >= 0 && gy >= 0 && gz >= 0 && gx < g[3] && gy < g[4] && gz < g[5]) {
                        int i = (gx * g[4] + gy) * g[5] + gz;
                        if ((bits[i >>> 6] & 1L << i) != 0) {
                            return true;
                        }
                    }
                }
            }
        }
        // the game leaves sections without blocks out of its list, but they can be seen into; outside the world: seen
        // (members are named on the class that declares them, so checking them reads few class files)
        LevelHeightAccessor height = level;
        for (int x = x0; x <= x1; x++) {
            for (int y = y0; y <= y1; y++) {
                if (height.isOutsideBuildHeight(y << 4)) {
                    return true;
                }
                for (int z = z0; z <= z1; z++) {
                    ChunkAccess chunk = ((Level) level).getChunk(x, z);
                    if (chunk.getSection(height.getSectionIndexFromSectionY(y)).hasOnlyAir()) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    /** The sections in view as bits, made again only when the game's list changed (checked once a frame). */
    private static Object[] blockforge$lite$view(List<?> list, Frustum frame) {
        int n = list.size();
        if (n == 0) {
            return null;
        }
        Object[] view = blockforge$lite$view;
        if (view != null && view[0] == list) {
            if (view[4] == frame) {
                return view;
            }
            Object[] copy = (Object[]) view[1];
            if (copy.length == n) {
                int i = 0;
                while (i < n && copy[i] == list.get(i)) {
                    i++;
                }
                if (i == n) {
                    view = new Object[] {list, copy, view[2], view[3], frame};
                    blockforge$lite$view = view;
                    return view;
                }
            }
        }
        Object[] copy = list.toArray();
        long[] nodes = new long[n];
        int minX = Integer.MAX_VALUE;
        int minY = Integer.MAX_VALUE;
        int minZ = Integer.MAX_VALUE;
        int maxX = Integer.MIN_VALUE;
        int maxY = Integer.MIN_VALUE;
        int maxZ = Integer.MIN_VALUE;
        for (int i = 0; i < n; i++) {
            long node = ((SectionRenderDispatcher.RenderSection) copy[i]).getSectionNode();
            nodes[i] = node;
            int x = SectionPos.x(node);
            int y = SectionPos.y(node);
            int z = SectionPos.z(node);
            minX = Math.min(minX, x);
            minY = Math.min(minY, y);
            minZ = Math.min(minZ, z);
            maxX = Math.max(maxX, x);
            maxY = Math.max(maxY, y);
            maxZ = Math.max(maxZ, z);
        }
        long cells = (long) (maxX - minX + 1) * (maxY - minY + 1) * (maxZ - minZ + 1);
        if (cells > (1L << 26)) {
            return null;
        }
        int[] g = {minX, minY, minZ, maxX - minX + 1, maxY - minY + 1, maxZ - minZ + 1};
        long[] bits = new long[(int) ((cells + 63) >>> 6)];
        for (int i = 0; i < n; i++) {
            long node = nodes[i];
            int c = ((SectionPos.x(node) - minX) * g[4] + SectionPos.y(node) - minY) * g[5] + SectionPos.z(node) - minZ;
            bits[c >>> 6] |= 1L << c;
        }
        view = new Object[] {list, copy, bits, g, frame};
        blockforge$lite$view = view;
        return view;
    }
}
