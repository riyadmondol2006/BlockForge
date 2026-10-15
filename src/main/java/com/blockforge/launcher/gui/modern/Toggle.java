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
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.event.KeyEvent;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.Ellipse2D;
import java.awt.geom.RoundRectangle2D;
import java.util.function.Consumer;

import javax.swing.AbstractAction;
import javax.swing.JComponent;
import javax.swing.KeyStroke;

/** An on/off switch. */
public class Toggle extends JComponent {
    private static final long serialVersionUID = 1L;

    private static final int W = 40;
    private static final int H = 22;

    private final String accessibleName;
    private boolean on;
    private boolean hover;
    private Consumer<Boolean> onChange = v -> {
    };

    public Toggle(boolean on, String accessibleName) {
        this.on = on;
        setOpaque(false);
        setFocusable(true);
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        setToolTipText(accessibleName);
        this.accessibleName = accessibleName;
        addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                flip();
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
        getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_SPACE, 0), "flip");
        getActionMap().put("flip", new AbstractAction() {
            private static final long serialVersionUID = 1L;

            @Override
            public void actionPerformed(java.awt.event.ActionEvent e) {
                flip();
            }
        });
    }

    @Override
    public javax.accessibility.AccessibleContext getAccessibleContext() {
        if (accessibleContext == null) {
            accessibleContext = new AccessibleJComponent() {
                private static final long serialVersionUID = 1L;

                @Override
                public javax.accessibility.AccessibleRole getAccessibleRole() {
                    return javax.accessibility.AccessibleRole.TOGGLE_BUTTON;
                }

                @Override
                public String getAccessibleName() {
                    return accessibleName;
                }

                @Override
                public javax.accessibility.AccessibleStateSet getAccessibleStateSet() {
                    javax.accessibility.AccessibleStateSet states = super.getAccessibleStateSet();
                    if (on) {
                        states.add(javax.accessibility.AccessibleState.CHECKED);
                    }
                    return states;
                }
            };
        }
        return accessibleContext;
    }

    private void flip() {
        if (!isEnabled()) {
            return;
        }
        on = !on;
        repaint();
        onChange.accept(on);
    }

    public Toggle onChange(Consumer<Boolean> onChange) {
        this.onChange = onChange;
        return this;
    }

    public boolean isOn() {
        return on;
    }

    /** Changes the switch without telling the listener. */
    public void setOn(boolean on) {
        this.on = on;
        repaint();
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(W + 4, H + 4);
    }

    @Override
    public Dimension getMaximumSize() {
        return getPreferredSize();
    }

    @Override
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setComposite(java.awt.AlphaComposite.SrcOver.derive(isEnabled() ? 1f : 0.45f));
        float x = (getWidth() - W) / 2f;
        float y = (getHeight() - H) / 2f;

        g.setColor(on ? (hover ? Palette.accentHover() : Palette.accent()) : (hover ? Palette.fieldHover()
            : Palette.field()));
        g.fill(new RoundRectangle2D.Float(x, y, W, H, H, H));
        if (!on) {
            g.setColor(Palette.fieldBorder());
            g.draw(new RoundRectangle2D.Float(x + 0.5f, y + 0.5f, W - 1, H - 1, H - 1, H - 1));
        }
        if (isFocusOwner() && FocusRing.visible()) {
            g.setColor(Palette.accent());
            g.setStroke(new java.awt.BasicStroke(2f));
            g.draw(new RoundRectangle2D.Float(x - 2, y - 2, W + 4, H + 4, H + 4, H + 4));
        }

        float knob = H - 6;
        float kx = on ? x + W - knob - 3 : x + 3;
        g.setColor(on ? Palette.onAccent() : Palette.textDim());
        g.fill(new Ellipse2D.Float(kx, y + 3, knob, knob));
        g.dispose();
    }
}
