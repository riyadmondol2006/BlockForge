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

import java.awt.Cursor;
import java.awt.Dimension;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.RoundRectangle2D;
import java.util.function.IntConsumer;

import javax.swing.JComponent;

/** A row of pill shaped choices where exactly one is selected. */
public class ChipRow extends JComponent {
    private static final long serialVersionUID = 1L;

    private final String[] labels;
    private int selected;

    public ChipRow(String[] labels, int selected, IntConsumer onSelect) {
        this.labels = labels;
        this.selected = selected;
        setOpaque(false);
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        setFont(Palette.bold(13f));

        addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                int index = indexAt(e.getX());
                if (index >= 0 && index != ChipRow.this.selected) {
                    ChipRow.this.selected = index;
                    repaint();
                    onSelect.accept(index);
                }
            }
        });
    }

    public int getSelected() {
        return selected;
    }

    private int chipWidth(FontMetrics fm, int i) {
        return fm.stringWidth(labels[i]) + 32;
    }

    private int indexAt(int x) {
        FontMetrics fm = getFontMetrics(Palette.bold(13f));
        int left = 0;
        for (int i = 0; i < labels.length; i++) {
            int w = chipWidth(fm, i);
            if (x >= left && x < left + w) {
                return i;
            }
            left += w + 8;
        }
        return -1;
    }

    @Override
    public Dimension getPreferredSize() {
        FontMetrics fm = getFontMetrics(Palette.bold(13f));
        int w = 0;
        for (int i = 0; i < labels.length; i++) {
            w += chipWidth(fm, i) + (i > 0 ? 8 : 0);
        }
        return new Dimension(w, 36);
    }

    @Override
    public Dimension getMaximumSize() {
        return getPreferredSize();
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
        g.setFont(Palette.bold(13f));
        FontMetrics fm = g.getFontMetrics();
        int h = getHeight();
        int x = 0;
        for (int i = 0; i < labels.length; i++) {
            int w = chipWidth(fm, i);
            boolean on = i == selected;
            g.setColor(on ? Palette.accentSoft() : Palette.field());
            g.fill(new RoundRectangle2D.Float(x, 0, w, h, h, h));
            if (on) {
                g.setColor(Palette.accent());
                g.setStroke(new java.awt.BasicStroke(1.5f));
                g.draw(new RoundRectangle2D.Float(x + 0.75f, 0.75f, w - 1.5f, h - 1.5f, h, h));
            }
            g.setColor(on ? Palette.accent() : Palette.textDim());
            g.drawString(labels[i], x + 16, (h - fm.getHeight()) / 2 + fm.getAscent());
            x += w + 8;
        }
        g.dispose();
    }
}
