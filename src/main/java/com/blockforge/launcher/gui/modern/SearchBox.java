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
import java.awt.Insets;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;

import javax.swing.JTextField;
import javax.swing.Timer;
import javax.swing.event.DocumentEvent;
import javax.swing.event.DocumentListener;

/**
 * Rounded text input with an optional leading search icon and a placeholder.
 */
public class SearchBox extends JTextField {
    private static final long serialVersionUID = 1L;

    /** corner diameter, matching TextComponent.arc and Component.arc in the Material themes */
    private static final float ARC = 20f;

    private final String placeholder;
    private boolean hover;
    private final boolean showIcon;
    private Timer debounce;

    public SearchBox(String placeholder, boolean showIcon, int columns) {
        super(columns);
        this.placeholder = placeholder;
        this.showIcon = showIcon;
        setOpaque(false);
        setBorder(javax.swing.BorderFactory.createEmptyBorder(0, showIcon ? 42 : 14, 0, 14));
        setFont(Palette.font(14f));
        setMargin(new Insets(0, 0, 0, 0));

        // repaint the outline on focus changes, and show long values from their start when not editing
        addFocusListener(new java.awt.event.FocusAdapter() {
            @Override
            public void focusGained(java.awt.event.FocusEvent e) {
                repaint();
            }

            @Override
            public void focusLost(java.awt.event.FocusEvent e) {
                setCaretPosition(0);
                repaint();
            }
        });
        addMouseListener(new java.awt.event.MouseAdapter() {
            @Override
            public void mouseEntered(java.awt.event.MouseEvent e) {
                hover = true;
                repaint();
            }

            @Override
            public void mouseExited(java.awt.event.MouseEvent e) {
                hover = false;
                repaint();
            }
        });
    }

    /** Gives a dropdown the same height, font and padding as this text field, for use next to one. */
    public static <T> javax.swing.JComboBox<T> styleLike(javax.swing.JComboBox<T> box, int width) {
        box.setFont(Palette.font(14f));
        box.setPreferredSize(new Dimension(width, 44));
        box.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
        return box;
    }

    @Override
    public void setText(String text) {
        super.setText(text);
        if (!isFocusOwner()) {
            setCaretPosition(0);
        }
    }

    /** Calls the action once the user pauses typing. */
    public void onChange(int delayMs, Runnable action) {
        debounce = new Timer(delayMs, e -> action.run());
        debounce.setRepeats(false);
        getDocument().addDocumentListener(new DocumentListener() {
            @Override
            public void insertUpdate(DocumentEvent e) {
                debounce.restart();
            }

            @Override
            public void removeUpdate(DocumentEvent e) {
                debounce.restart();
            }

            @Override
            public void changedUpdate(DocumentEvent e) {
                debounce.restart();
            }
        });
    }

    @Override
    public Dimension getPreferredSize() {
        Dimension d = super.getPreferredSize();
        return new Dimension(Math.max(d.width, 220), 44);
    }

    @Override
    public Dimension getMaximumSize() {
        return new Dimension(super.getMaximumSize().width, 44);
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        int w = getWidth();
        int h = getHeight();

        // same shape and colours as the theme's dropdowns: 10px corners, filled, soft outline, green when focused
        boolean focused = isFocusOwner();
        boolean hovered = !focused && hover;
        g.setColor(hovered ? Palette.fieldHover() : Palette.field());
        g.fill(new RoundRectangle2D.Float(0, 0, w, h, ARC, ARC));
        g.setColor(focused ? Palette.accent() : Palette.fieldBorder());
        g.setStroke(new java.awt.BasicStroke(focused ? 2f : 1f));
        float inset = focused ? 1f : 0.5f;
        g.draw(new RoundRectangle2D.Float(inset, inset, w - 2 * inset, h - 2 * inset, ARC - inset, ARC - inset));

        if (showIcon) {
            Icons.SEARCH.paint(g, 16, (h - 20) / 2f, 20, Palette.textDim());
        }

        setForeground(Palette.text());
        setCaretColor(Palette.accent());
        setSelectionColor(Palette.accentSoft());
        setSelectedTextColor(Palette.text());

        if (getText().isEmpty() && placeholder != null) {
            g.setFont(getFont());
            g.setColor(Palette.textDim());
            java.awt.FontMetrics fm = g.getFontMetrics();
            g.drawString(placeholder, getInsets().left, (h - fm.getHeight()) / 2 + fm.getAscent());
        }
        g.dispose();

        super.paintComponent(g0);
    }
}
