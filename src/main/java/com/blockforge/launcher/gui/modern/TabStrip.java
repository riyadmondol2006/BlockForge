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
import java.awt.event.KeyEvent;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.event.MouseMotionAdapter;
import java.awt.geom.RoundRectangle2D;
import java.util.function.IntConsumer;

import javax.swing.AbstractAction;
import javax.swing.JComponent;
import javax.swing.KeyStroke;

/** Big text tabs with an icon, a count and an accent bar under the selected one. */
public class TabStrip extends JComponent {
    private static final long serialVersionUID = 1L;

    private static final int GAP = 6;
    private static final int PAD = 16;
    private static final int ICON = 20;

    private final String[] labels;
    private final Icons[] icons;
    private final String[] counts;
    private final IntConsumer onSelect;
    private int selected;
    private int hover = -1;

    public TabStrip(String[] labels, Icons[] icons, IntConsumer onSelect) {
        this.labels = labels;
        this.icons = icons;
        this.counts = new String[labels.length];
        this.onSelect = onSelect;
        setOpaque(false);
        setFocusable(true);
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                select(indexAt(e.getX()));
            }

            @Override
            public void mouseExited(MouseEvent e) {
                hover = -1;
                repaint();
            }
        });
        addMouseMotionListener(new MouseMotionAdapter() {
            @Override
            public void mouseMoved(MouseEvent e) {
                int now = indexAt(e.getX());
                if (now != hover) {
                    hover = now;
                    repaint();
                }
            }
        });
        getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_LEFT, 0), "previous");
        getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_RIGHT, 0), "next");
        getActionMap().put("previous", new AbstractAction() {
            private static final long serialVersionUID = 1L;

            @Override
            public void actionPerformed(java.awt.event.ActionEvent e) {
                select(selected - 1);
            }
        });
        getActionMap().put("next", new AbstractAction() {
            private static final long serialVersionUID = 1L;

            @Override
            public void actionPerformed(java.awt.event.ActionEvent e) {
                select(selected + 1);
            }
        });
    }

    /** Selects a tab and tells the listener, as if it were clicked. */
    public void select(int index) {
        if (index >= 0 && index < labels.length && index != selected) {
            selected = index;
            repaint();
            onSelect.accept(index);
        }
    }

    public int getSelected() {
        return selected;
    }

    /** The small number shown next to a tab's name, or null for none. */
    public void setCount(int index, String count) {
        counts[index] = count;
        revalidate();
        repaint();
    }

    private int tabWidth(FontMetrics bold, FontMetrics small, int i) {
        int w = PAD * 2 + ICON + 8 + bold.stringWidth(labels[i]);
        if (counts[i] != null) {
            w += 8 + small.stringWidth(counts[i]) + 14;
        }
        return w;
    }

    private int indexAt(int x) {
        FontMetrics bold = getFontMetrics(Palette.bold(14f));
        FontMetrics small = getFontMetrics(Palette.bold(11f));
        int left = 0;
        for (int i = 0; i < labels.length; i++) {
            int w = tabWidth(bold, small, i);
            if (x >= left && x < left + w) {
                return i;
            }
            left += w + GAP;
        }
        return -1;
    }

    @Override
    public Dimension getPreferredSize() {
        FontMetrics bold = getFontMetrics(Palette.bold(14f));
        FontMetrics small = getFontMetrics(Palette.bold(11f));
        int w = 0;
        for (int i = 0; i < labels.length; i++) {
            w += tabWidth(bold, small, i) + (i > 0 ? GAP : 0);
        }
        return new Dimension(w, 48);
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
        FontMetrics bold = g.getFontMetrics(Palette.bold(14f));
        FontMetrics small = g.getFontMetrics(Palette.bold(11f));
        int h = getHeight();
        int x = 0;
        for (int i = 0; i < labels.length; i++) {
            int w = tabWidth(bold, small, i);
            boolean on = i == selected;
            if (!on && i == hover) {
                g.setColor(Palette.field());
                g.fill(new RoundRectangle2D.Float(x, 2, w, h - 8, 14, 14));
            }
            if (on) {
                g.setColor(Palette.accent());
                g.fill(new RoundRectangle2D.Float(x + PAD - 4, h - 4, w - 2 * PAD + 8, 4, 4, 4));
                if (isFocusOwner() && FocusRing.visible()) {
                    g.setStroke(new java.awt.BasicStroke(2f));
                    g.draw(new RoundRectangle2D.Float(x + 1, 3, w - 2, h - 10, 14, 14));
                }
            }
            java.awt.Color fg = on ? Palette.text() : Palette.textDim();
            int cy = (h - 4) / 2;
            int cx = x + PAD;
            icons[i].paint(g, cx, cy - ICON / 2f, ICON, on ? Palette.accent() : fg);
            cx += ICON + 8;
            g.setFont(Palette.bold(14f));
            g.setColor(fg);
            g.drawString(labels[i], cx, cy - bold.getHeight() / 2 + bold.getAscent());
            cx += bold.stringWidth(labels[i]);
            if (counts[i] != null) {
                cx += 8;
                int bw = small.stringWidth(counts[i]) + 14;
                g.setColor(on ? Palette.accentSoft() : Palette.field());
                g.fill(new RoundRectangle2D.Float(cx, cy - 10, bw, 20, 20, 20));
                g.setFont(Palette.bold(11f));
                g.setColor(on ? Palette.accent() : Palette.textDim());
                g.drawString(counts[i], cx + 7, cy - small.getHeight() / 2 + small.getAscent());
            }
            x += w + GAP;
        }
        g.dispose();
    }
}
