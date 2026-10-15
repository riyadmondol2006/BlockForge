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
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.Callable;
import java.util.function.Consumer;

import javax.imageio.ImageIO;
import javax.swing.ImageIcon;
import javax.swing.SwingUtilities;

import com.blockforge.launcher.App;
import com.blockforge.launcher.managers.LogManager;

/**
 * Loads and scales images off the UI thread and keeps recently used ones in memory.
 */
public final class ImageCache {
    private static final int MAX_ENTRIES = 160;

    private static final Map<String, BufferedImage> CACHE = Collections
        .synchronizedMap(new LinkedHashMap<String, BufferedImage>(64, 0.75f, true) {
            private static final long serialVersionUID = 1L;

            @Override
            protected boolean removeEldestEntry(Map.Entry<String, BufferedImage> eldest) {
                return size() > MAX_ENTRIES;
            }
        });

    private ImageCache() {
    }

    /**
     * Loads a remote image, scaled to cover the given size, and hands it to the callback on the UI thread. The
     * callback receives null if the image couldn't be loaded.
     */
    public static void load(String url, int width, int height, Consumer<BufferedImage> callback) {
        if (url == null || url.isEmpty()) {
            callback.accept(null);
            return;
        }

        String key = url + "@" + width + "x" + height;
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
                return source == null ? null : cover(source, width, height);
            }
        }, key, callback);
    }

    /** Runs a loader on a background thread and caches its result under the key. */
    public static void async(Callable<BufferedImage> loader, String key, Consumer<BufferedImage> callback) {
        BufferedImage cached = key == null ? null : CACHE.get(key);
        if (cached != null) {
            callback.accept(cached);
            return;
        }

        App.TASKPOOL.execute(() -> {
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
