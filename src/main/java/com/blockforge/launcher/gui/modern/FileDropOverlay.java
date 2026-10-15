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

import java.awt.BasicStroke;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Rectangle;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;

import javax.swing.JComponent;

/**
 * Covers a window while files are dragged over it: it says what dropping will do and, when the pointer is over an
 * instance, outlines that instance and names it.
 */
public class FileDropOverlay extends JComponent {
    private static final long serialVersionUID = 1L;

    private String title;
    private String subtitle;
    private Rectangle target;
    private String targetLabel;

    public FileDropOverlay(String title, String subtitle) {
        this.title = title;
        this.subtitle = subtitle;
        setOpaque(false);
    }

    public void setText(String title, String subtitle) {
        this.title = title;
        this.subtitle = subtitle;
        repaint();
    }

    /** Outlines an area (in this component's coordinates) with a label, or clears it with null. */
    public void setTarget(Rectangle target, String label) {
        boolean same = target == null ? this.target == null : target.equals(this.target);
        if (!same || (label != null && !label.equals(targetLabel))) {
            this.target = target;
            this.targetLabel = label;
            repaint();
        }
    }

    public Rectangle getTarget() {
        return target;
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
        int w = getWidth();
        int h = getHeight();

        if (target != null) {
            // keep the instance under the pointer bright so it's obvious where the files go
            java.awt.geom.Area dim = new java.awt.geom.Area(new Rectangle(0, 0, w, h));
            dim.subtract(new java.awt.geom.Area(new RoundRectangle2D.Float(target.x, target.y, target.width,
                target.height, 22, 22)));
            g.setColor(Palette.alpha(Palette.bg(), 215));
            g.fill(dim);
            g.setColor(Palette.accent());
            g.setStroke(new BasicStroke(3f));
            g.draw(new RoundRectangle2D.Float(target.x - 2, target.y - 2, target.width + 4, target.height + 4, 24,
                24));
            if (targetLabel != null) {
                // inside the card, over the top of its picture, so it never covers the card above
                g.setFont(Palette.fontFor(targetLabel, 13f, true));
                FontMetrics fm = g.getFontMetrics();
                int pw = Math.min(fm.stringWidth(targetLabel) + 28, Math.max(40, target.width - 20));
                int px = target.x + (target.width - pw) / 2;
                int py = target.y + 12;
                g.fill(new RoundRectangle2D.Float(px, py, pw, 30, 30, 30));
                g.setColor(Palette.onAccent());
                java.awt.Shape clip = g.getClip();
                g.clipRect(px + 10, py, pw - 20, 30);
                g.drawString(targetLabel, px + 14, py + (30 - fm.getHeight()) / 2 + fm.getAscent());
                g.setClip(clip);
            }
            g.dispose();
            return;
        }

        // strong enough that the cards underneath don't read through the message
        g.setColor(Palette.alpha(Palette.bg(), 238));
        g.fillRect(0, 0, w, h);
        g.setColor(Palette.accent());
        g.setStroke(new BasicStroke(2.5f, BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND, 10f,
            new float[] { 10f, 8f }, 0f));
        g.draw(new RoundRectangle2D.Float(18, 18, w - 36, h - 36, 28, 28));

        java.awt.Font titleFont = Palette.fontFor(title, 22f, true);
        java.awt.Font subFont = Palette.fontFor(subtitle, 14f, false);
        FontMetrics tfm = g.getFontMetrics(titleFont);
        FontMetrics sfm = g.getFontMetrics(subFont);
        int textW = Math.max(tfm.stringWidth(title), subtitle == null ? 0 : sfm.stringWidth(subtitle));
        int cw = Math.min(w - 80, textW + 64);
        int ch = subtitle == null ? 168 : 196;
        int cx = (w - cw) / 2;
        int cy = h / 2 - 116;
        // a solid card behind the message
        g.setColor(Palette.card());
        g.fill(new RoundRectangle2D.Float(cx, cy, cw, ch, 26, 26));
        g.setStroke(new BasicStroke(1f));
        g.setColor(Palette.outline());
        g.draw(new RoundRectangle2D.Float(cx + 0.5f, cy + 0.5f, cw - 1, ch - 1, 26, 26));

        Icons.IMPORT.paint(g, (w - 56) / 2f, h / 2f - 92, 56, Palette.accent());
        g.setFont(titleFont);
        g.setColor(Palette.text());
        g.drawString(title, (w - tfm.stringWidth(title)) / 2, h / 2 + 4);
        if (subtitle != null) {
            g.setFont(subFont);
            g.setColor(Palette.textDim());
            g.drawString(subtitle, (w - sfm.stringWidth(subtitle)) / 2, h / 2 + 34);
        }
        g.dispose();
    }
}
