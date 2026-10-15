/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */
package com.blockforge.launcher.gui.modern;

import java.awt.Graphics2D;
import java.awt.Image;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.net.URL;
import java.net.URLConnection;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.function.Consumer;

import javax.imageio.ImageIO;
import javax.swing.ImageIcon;
import javax.swing.SwingUtilities;

import com.blockforge.launcher.managers.LogManager;

/**
 * Loads and scales images off the UI thread and keeps recently used ones in memory.
 */
public final class ImageCache {
    /**
     * How much decoded image data is kept. A banner for a big instance card is over half a megabyte once decoded, so
     * the limit is on bytes rather than on the number of pictures.
     */
    static final long MAX_BYTES = com.blockforge.launcher.utils.LightMode.imageCacheBytes();

    private static final Lru CACHE = new Lru(MAX_BYTES);

    static {
        // on a computer with little memory, pictures are dropped while Minecraft runs; they reload when shown again
        com.blockforge.launcher.mclauncher.LauncherMemory.register(CACHE::clear);
    }

    /**
     * Images are loaded on their own threads: a page of modpack icons from a slow server would otherwise queue up in
     * front of the launcher's other background work (searches, log uploads, play time reports).
     */
    private static final ExecutorService LOADER = loaderPool();

    /** The loader threads end after 30 seconds without pictures to load, so an idle launcher doesn't keep them. */
    private static ExecutorService loaderPool() {
        int threads = com.blockforge.launcher.utils.LightMode.imageLoaderThreads();
        java.util.concurrent.ThreadPoolExecutor pool = new java.util.concurrent.ThreadPoolExecutor(threads, threads,
            30, java.util.concurrent.TimeUnit.SECONDS, new java.util.concurrent.LinkedBlockingQueue<>(), r -> {
                Thread thread = new Thread(r, "image-loader");
                thread.setDaemon(true);
                return thread;
            });
        pool.allowCoreThreadTimeOut(true);
        return pool;
    }

    private static volatile int pixelScale;

    private ImageCache() {
    }

    /**
     * How many image pixels each screen point needs on the sharpest screen attached: 1 on ordinary screens, 2 on high
     * density ones. Pictures are kept at that size, so an ordinary screen doesn't hold four times the pixels it shows.
     */
    public static int pixelScale() {
        if (pixelScale == 0) {
            pixelScale = clampScale(maxScreenScale());
        }
        return pixelScale;
    }

    /** Pixels per point rounded up and kept between 1 and 2; unknown (0 or less) counts as 2, as before. */
    static int clampScale(double scale) {
        if (!(scale > 0)) {
            return 2;
        }
        return (int) Math.max(1, Math.min(2, Math.ceil(scale - 0.01)));
    }

    private static double maxScreenScale() {
        try {
            double max = 0;
            for (java.awt.GraphicsDevice device : java.awt.GraphicsEnvironment.getLocalGraphicsEnvironment()
                .getScreenDevices()) {
                max = Math.max(max, device.getDefaultConfiguration().getDefaultTransform().getScaleX());
            }
            return max;
        } catch (Throwable e) {
            // headless or no screens
            return 0;
        }
    }

    /** The image size in pixels for a picture shown at the given size in points. */
    public static int px(int points) {
        return points * pixelScale();
    }

    /** Least recently used images go first once the total size is over the limit. */
    static final class Lru {
        private final long maxBytes;
        private final LinkedHashMap<String, BufferedImage> images = new LinkedHashMap<>(64, 0.75f, true);
        private long bytes;

        Lru(long maxBytes) {
            this.maxBytes = maxBytes;
        }

        synchronized BufferedImage get(String key) {
            return images.get(key);
        }

        synchronized void put(String key, BufferedImage image) {
            BufferedImage old = images.put(key, image);
            if (old != null) {
                bytes -= sizeOf(old);
            }
            bytes += sizeOf(image);

            Iterator<Map.Entry<String, BufferedImage>> eldest = images.entrySet().iterator();
            // the image just added always stays, even if it is bigger than the whole limit on its own
            while (bytes > maxBytes && images.size() > 1 && eldest.hasNext()) {
                Map.Entry<String, BufferedImage> entry = eldest.next();
                if (entry.getKey().equals(key)) {
                    continue;
                }
                bytes -= sizeOf(entry.getValue());
                eldest.remove();
            }
        }

        synchronized long bytes() {
            return bytes;
        }

        synchronized void clear() {
            images.clear();
            bytes = 0;
        }

        synchronized int size() {
            return images.size();
        }

        static long sizeOf(BufferedImage image) {
            return (long) image.getWidth() * image.getHeight() * 4;
        }
    }

    /**
     * Loads a remote image, scaled to cover the given size, and hands it to the callback on the UI thread. The
     * callback receives null if the image couldn't be loaded.
     */
    public static void load(String url, int width, int height, Consumer<BufferedImage> callback) {
        load(url, width, height, false, callback);
    }

    /**
     * Like {@link #load(String, int, int, Consumer)}, but with {@code whole} set the image is scaled to fit inside the
     * area instead of covering it, so nothing is cut off; the space left over is transparent.
     */
    public static void load(String url, int width, int height, boolean whole, Consumer<BufferedImage> callback) {
        if (url == null || url.isEmpty()) {
            callback.accept(null);
            return;
        }

        String key = url + "@" + width + "x" + height + (whole ? "fit" : "");
        BufferedImage cached = CACHE.get(key);
        if (cached != null) {
            callback.accept(cached);
            return;
        }

        async(() -> {
            URLConnection connection = new URL(url).openConnection();
            connection.setRequestProperty("User-Agent", "BlockForge");
            connection.setConnectTimeout(10000);
            connection.setReadTimeout(15000);
            try (java.io.InputStream in = connection.getInputStream()) {
                BufferedImage source = ImageIO.read(in);
                return source == null ? null : whole ? fit(source, width, height) : cover(source, width, height);
            }
        }, key, callback);
    }

    /** Scales the image to fit inside width x height, centred, keeping all of it; the rest stays transparent. */
    public static BufferedImage fit(BufferedImage src, int width, int height) {
        BufferedImage out = new BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB);
        Graphics2D g = out.createGraphics();
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
        g.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY);
        double scale = Math.min(width / (double) src.getWidth(), height / (double) src.getHeight());
        int w = Math.max(1, (int) Math.round(src.getWidth() * scale));
        int h = Math.max(1, (int) Math.round(src.getHeight() * scale));
        g.drawImage(src, (width - w) / 2, (height - h) / 2, w, h, null);
        g.dispose();
        return out;
    }

    /** Runs a loader on a background thread and caches its result under the key. */
    public static void async(Callable<BufferedImage> loader, String key, Consumer<BufferedImage> callback) {
        BufferedImage cached = key == null ? null : CACHE.get(key);
        if (cached != null) {
            callback.accept(cached);
            return;
        }

        LOADER.execute(() -> {
            BufferedImage result = null;
            try {
                result = loader.call();
            } catch (Exception e) {
                LogManager.debug("Couldn't load image " + key + ": " + e.getMessage());
            }

            BufferedImage finalResult = result;
            if (finalResult != null && key != null) {
                CACHE.put(key, finalResult);
            }
            SwingUtilities.invokeLater(() -> callback.accept(finalResult));
        });
    }

    /** Scales and crops the image so that it fills the whole width x height area. */
    public static BufferedImage cover(BufferedImage src, int width, int height) {
        BufferedImage out = new BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB);
        Graphics2D g = out.createGraphics();
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
        g.setRenderingHint(RenderingHints.KEY_RENDERING, RenderingHints.VALUE_RENDER_QUALITY);
        double scale = Math.max(width / (double) src.getWidth(), height / (double) src.getHeight());
        int w = (int) Math.ceil(src.getWidth() * scale);
        int h = (int) Math.ceil(src.getHeight() * scale);
        g.drawImage(src, (width - w) / 2, (height - h) / 2, w, h, null);
        g.dispose();
        return out;
    }

    public static BufferedImage toBuffered(ImageIcon icon) {
        if (icon == null || icon.getIconWidth() <= 0) {
            return null;
        }
        if (icon.getImage() instanceof BufferedImage) {
            return (BufferedImage) icon.getImage();
        }
        BufferedImage out = new BufferedImage(icon.getIconWidth(), icon.getIconHeight(),
            BufferedImage.TYPE_INT_ARGB);
        Graphics2D g = out.createGraphics();
        Image image = icon.getImage();
        g.drawImage(image, 0, 0, null);
        g.dispose();
        return out;
    }
}
