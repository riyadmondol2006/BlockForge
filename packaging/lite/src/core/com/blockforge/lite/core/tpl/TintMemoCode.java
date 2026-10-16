package com.blockforge.lite.core.tpl;

import java.lang.ref.WeakReference;
import java.util.Arrays;

/**
 * Template of the memory tint-memo adds to ClientLevel (uses only the JDK, so it fits every version and name set).
 * Never loaded: {@code Template} copies these members into the game class; see {@code TintMemo}.
 *
 * Each thread keeps the biomes it looked up last, in a table of 32 x 8 x 16 block positions (x, z, y), one slot per
 * position modulo that size, with the area the stored positions span. The game says when a chunk arrived or changed its
 * biomes (with its position) and when it cleared all its colours; a thread forgets its table when a chunk arrived
 * within 8 blocks of that area (the fuzzy biome lookup reads at most 6 blocks around a position), when all colours were
 * cleared, or when it is asked about another level.
 */
final class TintMemoCode {
    private TintMemoCode() {}

    /** Positions of the chunks that arrived, (x << 32 | z &amp; 0xffffffff), the last 256 of them. */
    private static long[] blockforge$lite$tintRing;
    private static volatile int blockforge$lite$tintLoads;
    private static volatile int blockforge$lite$tintClears;
    /**
     * Per thread: {int[] keys (x, y, z), Object[] biomes, WeakReference level,
     * int[] {clears, loads, minX, maxX, minZ, maxZ}}.
     */
    private static volatile ThreadLocal<Object[]> blockforge$lite$tintMemo;

    /** A chunk arrived or changed its biomes (called on the game's thread before it clears that chunk's colours). */
    static void blockforge$lite$tintChunk(int x, int z) {
        long[] ring = blockforge$lite$tintRing;
        if (ring == null) {
            ring = new long[256];
            blockforge$lite$tintRing = ring;
        }
        int n = blockforge$lite$tintLoads;
        ring[n & 255] = (long) x << 32 | z & 0xFFFFFFFFL;
        blockforge$lite$tintLoads = n + 1;
    }

    /** The game cleared all colours (resource reload). */
    static void blockforge$lite$tintClear() {
        blockforge$lite$tintClears = blockforge$lite$tintClears + 1;
    }

    /** The remembered biome of this level at x, y, z, or null. */
    static Object blockforge$lite$tintGet(Object level, int x, int y, int z) {
        Object[] t = blockforge$lite$tintTable(level);
        int i = (x & 31) | (z & 7) << 5 | (y & 15) << 8;
        Object v = ((Object[]) t[1])[i];
        int[] keys = (int[]) t[0];
        return v != null && keys[3 * i] == x && keys[3 * i + 1] == y && keys[3 * i + 2] == z ? v : null;
    }

    /** Remembers a biome this thread just looked up (right after tintGet on the same thread). */
    static void blockforge$lite$tintPut(int x, int y, int z, Object biome) {
        // two threads may both have made the ThreadLocal at the very start; the one that lost has no table here
        ThreadLocal<Object[]> tl = blockforge$lite$tintMemo;
        Object[] t = tl == null ? null : tl.get();
        if (t == null) {
            return;
        }
        int i = (x & 31) | (z & 7) << 5 | (y & 15) << 8;
        int[] keys = (int[]) t[0];
        keys[3 * i] = x;
        keys[3 * i + 1] = y;
        keys[3 * i + 2] = z;
        ((Object[]) t[1])[i] = biome;
        int[] m = (int[]) t[3];
        m[2] = Math.min(m[2], x);
        m[3] = Math.max(m[3], x);
        m[4] = Math.min(m[4], z);
        m[5] = Math.max(m[5], z);
    }

    /** This thread's table, emptied first when it may hold something the game changed since. */
    private static Object[] blockforge$lite$tintTable(Object level) {
        ThreadLocal<Object[]> tl = blockforge$lite$tintMemo;
        if (tl == null) {
            tl = new ThreadLocal<Object[]>();
            blockforge$lite$tintMemo = tl;
        }
        Object[] t = tl.get();
        int clears = blockforge$lite$tintClears;
        int loads = blockforge$lite$tintLoads;
        if (t == null) {
            t = new Object[] {new int[3 * 4096], new Object[4096], null, new int[6]};
            tl.set(t);
            blockforge$lite$tintReset(t, level, clears, loads);
            return t;
        }
        int[] m = (int[]) t[3];
        if (((WeakReference<?>) t[2]).get() != level || m[0] != clears) {
            blockforge$lite$tintReset(t, level, clears, loads);
        } else if (m[1] != loads) {
            boolean near = loads - m[1] > 128;
            long[] ring = blockforge$lite$tintRing;
            for (int s = m[1]; !near && s != loads; s++) {
                long c = ring[s & 255];
                int bx = (int) (c >> 32) << 4;
                int bz = (int) c << 4;
                near = bx - 8 <= m[3] && bx + 23 >= m[2] && bz - 8 <= m[5] && bz + 23 >= m[4];
            }
            // the ring entries read could have been written over meanwhile only after 128 more chunks
            if (near || blockforge$lite$tintLoads - m[1] > 128) {
                blockforge$lite$tintReset(t, level, clears, loads);
            } else {
                m[1] = loads;
            }
        }
        return t;
    }

    private static void blockforge$lite$tintReset(Object[] t, Object level, int clears, int loads) {
        Arrays.fill((Object[]) t[1], null);
        t[2] = new WeakReference<Object>(level);
        int[] m = (int[]) t[3];
        m[0] = clears;
        m[1] = loads;
        m[2] = Integer.MAX_VALUE;
        m[3] = Integer.MIN_VALUE;
        m[4] = Integer.MAX_VALUE;
        m[5] = Integer.MIN_VALUE;
    }
}
