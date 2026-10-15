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

import java.awt.Color;
import java.awt.Component;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.LayoutManager;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.RoundRectangle2D;
import java.util.function.Supplier;

import javax.swing.JComponent;
import javax.swing.JPanel;

/**
 * A panel with rounded corners whose colours are resolved when painting, so it follows the active theme.
 */
public class RoundPanel extends JPanel {
    private static final long serialVersionUID = 1L;

    private final int arc;
    private final Supplier<Color> fill;
    private Supplier<Color> hoverFill;
    private Supplier<Color> line;

    public RoundPanel(LayoutManager layout, int arc, Supplier<Color> fill) {
        super(layout);
        this.arc = arc;
        this.fill = fill;
        setOpaque(false);
    }

    public RoundPanel line(Supplier<Color> line) {
        this.line = line;
        return this;
    }

    /** Fills with a different colour while the pointer is anywhere over this panel or its children. */
    public RoundPanel hover(Supplier<Color> hoverFill) {
        this.hoverFill = hoverFill;
        this.hoverTracker = new MouseAdapter() {
            @Override
            public void mouseEntered(MouseEvent e) {
                repaint();
            }

            @Override
            public void mouseExited(MouseEvent e) {
                repaint();
            }
        };
        track(this, hoverTracker);
        return this;
    }

    @Override
    public Component add(Component comp) {
        Component c = super.add(comp);
        retrack(c);
        return c;
    }

    @Override
    public void add(Component comp, Object constraints) {
        super.add(comp, constraints);
        retrack(comp);
    }

    private MouseAdapter hoverTracker;

    private void retrack(Component c) {
        if (hoverTracker != null) {
            track(c, hoverTracker);
        }
    }

    private static void track(Component c, MouseAdapter tracker) {
        c.removeMouseListener(tracker);
        c.addMouseListener(tracker);
        if (c instanceof java.awt.Container) {
            for (Component child : ((java.awt.Container) c).getComponents()) {
                track(child, tracker);
            }
        }
    }

    public boolean isHovered() {
        return isShowing() && getMousePosition(true) != null;
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setColor(hoverFill != null && isHovered() ? hoverFill.get() : fill.get());
        g.fill(new RoundRectangle2D.Float(0, 0, getWidth(), getHeight(), arc, arc));
        if (line != null) {
            g.setColor(line.get());
            g.draw(new RoundRectangle2D.Float(0.5f, 0.5f, getWidth() - 1, getHeight() - 1, arc, arc));
        }
        g.dispose();
    }

    /** Shrinks the panel's content area; handy for card padding. */
    public RoundPanel pad(int top, int left, int bottom, int right) {
        setBorder(javax.swing.BorderFactory.createEmptyBorder(top, left, bottom, right));
        return this;
    }

    public static void setTransparent(JComponent c) {
        c.setOpaque(false);
    }
}
