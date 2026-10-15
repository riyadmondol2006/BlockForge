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

import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;
import java.awt.image.BufferedImage;

import javax.swing.JComponent;

/**
 * Draws the front of a Minecraft player from a skin file, with the outer layer (hat, jacket, sleeves) on top.
 */
public class SkinPreview extends JComponent {
    private static final long serialVersionUID = 1L;

    /** the figure is 16 skin pixels wide and 32 tall */
    private static final int FIGURE_W = 16;
    private static final int FIGURE_H = 32;

    private BufferedImage skin;
    private boolean slim;

    public SkinPreview() {
        setOpaque(false);
    }

    public void setSkin(BufferedImage skin, boolean slim) {
        this.skin = skin;
        this.slim = slim;
        repaint();
    }

    /** True for the sizes Minecraft accepts: 64x64, or the old 64x32. */
    public static boolean isValidSkin(BufferedImage image) {
        return image != null && image.getWidth() == 64 && (image.getHeight() == 64 || image.getHeight() == 32);
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(180, 300);
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setColor(Palette.field());
        g.fill(new RoundRectangle2D.Float(0, 0, getWidth(), getHeight(), 24, 24));

        if (skin == null) {
            Icons.PERSON.paint(g, getWidth() / 2f - 32, getHeight() / 2f - 32, 64, Palette.textDim());
            g.dispose();
            return;
        }

        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_OFF);
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
        int scale = Math.max(1, Math.min((getWidth() - 24) / FIGURE_W, (getHeight() - 24) / FIGURE_H));
        int ox = (getWidth() - FIGURE_W * scale) / 2;
        int oy = (getHeight() - FIGURE_H * scale) / 2;
        boolean modern = skin.getHeight() == 64;
        int arm = slim && modern ? 3 : 4;

        // inner layer: head, body, arms, legs. The player's right side is on the viewer's left.
        part(g, 8, 8, 8, 8, ox + 4 * scale, oy, scale, false);
        part(g, 20, 20, 8, 12, ox + 4 * scale, oy + 8 * scale, scale, false);
        part(g, 44, 20, arm, 12, ox + (4 - arm) * scale, oy + 8 * scale, scale, false);
        if (modern) {
            part(g, 36, 52, arm, 12, ox + 12 * scale, oy + 8 * scale, scale, false);
        } else {
            part(g, 44, 20, arm, 12, ox + 12 * scale, oy + 8 * scale, scale, true);
        }
        part(g, 4, 20, 4, 12, ox + 4 * scale, oy + 20 * scale, scale, false);
        if (modern) {
            part(g, 20, 52, 4, 12, ox + 8 * scale, oy + 20 * scale, scale, false);
        } else {
            part(g, 4, 20, 4, 12, ox + 8 * scale, oy + 20 * scale, scale, true);
        }

        // outer layer
        part(g, 40, 8, 8, 8, ox + 4 * scale, oy, scale, false);
        if (modern) {
            part(g, 20, 36, 8, 12, ox + 4 * scale, oy + 8 * scale, scale, false);
            part(g, 44, 36, arm, 12, ox + (4 - arm) * scale, oy + 8 * scale, scale, false);
            part(g, 52, 52, arm, 12, ox + 12 * scale, oy + 8 * scale, scale, false);
            part(g, 4, 36, 4, 12, ox + 4 * scale, oy + 20 * scale, scale, false);
            part(g, 4, 52, 4, 12, ox + 8 * scale, oy + 20 * scale, scale, false);
        }
        g.dispose();
    }

    private void part(Graphics2D g, int sx, int sy, int w, int h, int dx, int dy, int scale, boolean mirror) {
        if (sx + w > skin.getWidth() || sy + h > skin.getHeight()) {
            return;
        }
        if (mirror) {
            g.drawImage(skin, dx + w * scale, dy, dx, dy + h * scale, sx, sy, sx + w, sy + h, null);
        } else {
            g.drawImage(skin, dx, dy, dx + w * scale, dy + h * scale, sx, sy, sx + w, sy + h, null);
        }
    }
}
