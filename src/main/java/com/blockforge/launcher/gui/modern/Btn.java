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
import java.awt.Cursor;
import java.awt.Dimension;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Insets;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;

import javax.swing.JButton;

/**
 * Pill shaped button. Everything is painted here so the look is the same whichever Swing theme is active.
 */
public class Btn extends JButton {
    private static final long serialVersionUID = 1L;

    public enum Kind {
        /** filled with the accent colour, for the one main action */
        FILLED,
        /** soft filled, for secondary actions */
        TONAL,
        /** no background until hovered */
        TEXT,
        /** soft red, for destructive actions */
        DANGER
    }

    private final Kind kind;
    private Icons icon;
    private boolean iconOnly;
    private int height = 40;

    public Btn(String text, Kind kind) {
        super(text);
        this.kind = kind;
        init();
    }

    public static Btn iconButton(Icons icon, String tooltip) {
        Btn b = new Btn("", Kind.TEXT);
        b.icon = icon;
        b.iconOnly = true;
        b.height = 36;
        b.setToolTipText(tooltip);
        b.getAccessibleContext().setAccessibleName(tooltip);
        return b;
    }

    private void init() {
        setContentAreaFilled(false);
        setBorderPainted(false);
        setFocusPainted(false);
        setOpaque(false);
        setRolloverEnabled(true);
        setMargin(new Insets(0, 0, 0, 0));
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        setFont(Palette.bold(13f));
    }

    public Btn icon(Icons icon) {
        this.icon = icon;
        return this;
    }

    /** a smaller button for dense rows */
    public Btn compact() {
        this.height = 32;
        return this;
    }

    @Override
    public Dimension getPreferredSize() {
        FontMetrics fm = getFontMetrics(Palette.fontFor(getText(), 13f, true));
        if (iconOnly) {
            return new Dimension(height, height);
        }

        int w = 40;
        if (icon != null) {
            w += 18 + 8;
        }
        w += fm.stringWidth(getText());
        if (height < 40) {
            w -= 8;
        }
        return new Dimension(Math.max(w, 64), height);
    }

    @Override
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }

    @Override
    public Dimension getMaximumSize() {
        return getPreferredSize();
    }

    private Color background() {
        boolean pressed = getModel().isPressed();
        boolean hover = getModel().isRollover();

        if (kind == Kind.FILLED && !isEnabled()) {
            // a faded accent fill reads as white on pale green in the light theme; a neutral fill stays legible
            return Palette.field();
        }
        switch (kind) {
            case FILLED:
                return pressed ? Palette.accentPressed() : hover ? Palette.accentHover() : Palette.accent();
            case TONAL:
                return hover || pressed ? Palette.fieldHover() : Palette.field();
            case DANGER:
                return Palette.mix(Palette.card(), Palette.danger(), hover || pressed ? 0.32f : 0.2f);
            case TEXT:
            default:
                return hover || pressed ? Palette.field() : new Color(0, 0, 0, 0);
        }
    }

    private Color foreground() {
        if (kind == Kind.FILLED && !isEnabled()) {
            return Palette.textDim();
        }
        switch (kind) {
            case FILLED:
                return Palette.onAccent();
            case DANGER:
                return Palette.danger();
            case TEXT:
                return iconOnly ? (getModel().isRollover() ? Palette.text() : Palette.textDim()) : Palette.accent();
            case TONAL:
            default:
                return Palette.text();
        }
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);

        int w = getWidth();
        int h = getHeight();
        float alpha = isEnabled() || kind == Kind.FILLED ? 1f : 0.4f;
        g.setComposite(java.awt.AlphaComposite.SrcOver.derive(alpha));

        g.setColor(background());
        g.fill(new RoundRectangle2D.Float(0, 0, w, h, h, h));

        if (isFocusOwner() && FocusRing.visible() && !getModel().isPressed()) {
            g.setColor(Palette.accent());
            g.setStroke(new java.awt.BasicStroke(2f));
            g.draw(new RoundRectangle2D.Float(1, 1, w - 2, h - 2, h - 2, h - 2));
        }

        Color fg = foreground();
        g.setFont(Palette.fontFor(getText(), 13f, true));
        FontMetrics fm = g.getFontMetrics();

        if (iconOnly) {
            float s = 20;
            icon.paint(g, (w - s) / 2f, (h - s) / 2f, s, fg);
        } else {
            int textW = fm.stringWidth(getText());
            int contentW = textW + (icon != null ? 18 + 8 : 0);
            int x = (w - contentW) / 2;
            if (icon != null) {
                icon.paint(g, x, (h - 18) / 2f, 18, fg);
                x += 18 + 8;
            }
            g.setColor(fg);
            g.drawString(getText(), x, (h - fm.getHeight()) / 2 + fm.getAscent());
        }

        g.dispose();
    }
}
