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
import java.awt.geom.RoundRectangle2D;

import javax.swing.JComponent;
import javax.swing.KeyStroke;

/** A single pill shaped toggle: used for filters (any number on) and for one-of-many choices. */
public class Chip extends JComponent {
    private static final long serialVersionUID = 1L;

    private final String label;
    private boolean selected;
    private boolean hover;
    private Runnable onClick = () -> {
    };

    public Chip(String label) {
        this.label = label;
        setOpaque(false);
        setFocusable(true);
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        setFont(Palette.bold(13f));
        addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                if (isEnabled()) {
                    onClick.run();
                }
            }

            @Override
            public void mouseEntered(MouseEvent e) {
                hover = true;
                repaint();
            }

            @Override
            public void mouseExited(MouseEvent e) {
                hover = false;
                repaint();
            }
        });
        getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_SPACE, 0), "press");
        getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_ENTER, 0), "press");
        getActionMap().put("press", new javax.swing.AbstractAction() {
            private static final long serialVersionUID = 1L;

            @Override
            public void actionPerformed(java.awt.event.ActionEvent e) {
                if (isEnabled()) {
                    onClick.run();
                }
            }
        });
    }

    @Override
    public javax.accessibility.AccessibleContext getAccessibleContext() {
        if (accessibleContext == null) {
            accessibleContext = new AccessibleJComponent() {
                @Override
                public javax.accessibility.AccessibleRole getAccessibleRole() {
                    return javax.accessibility.AccessibleRole.TOGGLE_BUTTON;
                }

                @Override
                public String getAccessibleName() {
                    return label;
                }
            };
        }
        return accessibleContext;
    }

    public Chip onClick(Runnable onClick) {
        this.onClick = onClick;
        return this;
    }

    public void setSelected(boolean selected) {
        this.selected = selected;
        repaint();
    }

    public boolean isSelected() {
        return selected;
    }

    @Override
    public Dimension getPreferredSize() {
        FontMetrics fm = getFontMetrics(Palette.bold(13f));
        return new Dimension(fm.stringWidth(label) + 32, 36);
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
        int w = getWidth();
        int h = getHeight();
        g.setComposite(java.awt.AlphaComposite.SrcOver.derive(isEnabled() ? 1f : 0.4f));

        g.setColor(selected ? Palette.accentSoft() : hover ? Palette.fieldHover() : Palette.field());
        g.fill(new RoundRectangle2D.Float(0, 0, w, h, h, h));
        if (selected) {
            g.setColor(Palette.accent());
            g.setStroke(new java.awt.BasicStroke(1.5f));
            g.draw(new RoundRectangle2D.Float(0.75f, 0.75f, w - 1.5f, h - 1.5f, h, h));
        }
        if (isFocusOwner() && FocusRing.visible()) {
            g.setColor(Palette.accent());
            g.setStroke(new java.awt.BasicStroke(2f));
            g.draw(new RoundRectangle2D.Float(1, 1, w - 2, h - 2, h - 2, h - 2));
        }

        g.setFont(Palette.bold(13f));
        FontMetrics fm = g.getFontMetrics();
        g.setColor(selected ? Palette.accent() : Palette.textDim());
        g.drawString(label, 16, (h - fm.getHeight()) / 2 + fm.getAscent());
        g.dispose();
    }
}
