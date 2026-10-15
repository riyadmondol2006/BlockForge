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

import org.mini2Dx.gettext.GetText;

/**
 * The Install button on a search result. It turns into a progress bar while installing and shows when the thing is
 * installed, so the player never has to look anywhere else to see what happened.
 */
public class InstallButton extends JButton {
    private static final long serialVersionUID = 1L;

    public enum State {
        READY, QUEUED, WORKING, INSTALLED, FAILED
    }

    private State state = State.READY;
    private double fraction = -1;
    private String working = "";
    private int phase;
    private final javax.swing.Timer pulse;
    private final int width;

    public InstallButton(int width) {
        this.width = width;
        setContentAreaFilled(false);
        setBorderPainted(false);
        setFocusPainted(false);
        setOpaque(false);
        setRolloverEnabled(true);
        setMargin(new Insets(0, 0, 0, 0));
        setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        pulse = new javax.swing.Timer(com.blockforge.launcher.utils.LightMode.frameDelayMs() * 2, e -> {
            phase = (phase + 3) % 100;
            repaint();
        });
        setState(State.READY);
    }

    public State getState() {
        return state;
    }

    public void setState(State state) {
        this.state = state;
        if (state != State.WORKING) {
            fraction = -1;
        }
        setCursor(Cursor.getPredefinedCursor(state == State.READY || state == State.FAILED ? Cursor.HAND_CURSOR
            : Cursor.DEFAULT_CURSOR));
        String text;
        switch (state) {
            case QUEUED:
                text = GetText.tr("Waiting...");
                break;
            case WORKING:
                text = GetText.tr("Installing...");
                break;
            case INSTALLED:
                text = GetText.tr("Installed");
                break;
            case FAILED:
                text = GetText.tr("Try again");
                break;
            default:
                text = GetText.tr("Install");
                break;
        }
        setText(text);
        getAccessibleContext().setAccessibleName(text);
        if (state == State.WORKING && fraction < 0 && isShowing()) {
            pulse.start();
        } else if (state != State.WORKING) {
            pulse.stop();
        }
        repaint();
    }

    /** Shows how far the install is, from 0 to 1, or below 0 when that isn't known. */
    public void setProgress(double fraction, String label) {
        this.fraction = fraction;
        if (label != null) {
            this.working = label;
        }
        if (fraction < 0 && state == State.WORKING && isShowing()) {
            pulse.start();
        } else {
            pulse.stop();
        }
        repaint();
    }

    @Override
    public void removeNotify() {
        pulse.stop();
        super.removeNotify();
    }

    @Override
    public void addNotify() {
        super.addNotify();
        if (state == State.WORKING && fraction < 0) {
            pulse.start();
        }
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(width, 34);
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
        g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
        int w = getWidth();
        int h = getHeight();
        boolean hover = getModel().isRollover();
        RoundRectangle2D shape = new RoundRectangle2D.Float(0, 0, w, h, h, h);

        Color fg;
        Icons icon = null;
        String text = getText();
        switch (state) {
            case WORKING: {
                g.setColor(Palette.field());
                g.fill(shape);
                g.clip(shape);
                g.setColor(Palette.alpha(Palette.accent(), 110));
                if (fraction >= 0) {
                    g.fillRect(0, 0, (int) Math.round(w * fraction), h);
                    text = Math.round(fraction * 100) + "%";
                } else {
                    int band = w / 3;
                    int x = (int) ((w + band) * (phase / 100.0)) - band;
                    g.fillRect(x, 0, band, h);
                }
                g.setClip(null);
                fg = Palette.text();
                break;
            }
            case QUEUED:
                g.setColor(Palette.field());
                g.fill(shape);
                fg = Palette.textDim();
                break;
            case INSTALLED:
                g.setColor(Palette.accentSoft());
                g.fill(shape);
                fg = Palette.accent();
                icon = Icons.CHECK;
                break;
            case FAILED:
                g.setColor(Palette.mix(Palette.card(), Palette.danger(), hover ? 0.32f : 0.2f));
                g.fill(shape);
                fg = Palette.danger();
                icon = Icons.REFRESH;
                break;
            default:
                g.setColor(getModel().isPressed() ? Palette.accentPressed()
                    : hover ? Palette.accentHover() : Palette.accent());
                g.fill(shape);
                fg = Palette.onAccent();
                icon = Icons.DOWNLOAD;
                break;
        }

        if (isFocusOwner() && FocusRing.visible()) {
            g.setColor(Palette.accent());
            g.setStroke(new java.awt.BasicStroke(2f));
            g.draw(new RoundRectangle2D.Float(1, 1, w - 2, h - 2, h - 2, h - 2));
        }

        g.setFont(Palette.bold(13f));
        FontMetrics fm = g.getFontMetrics();
        int tw = fm.stringWidth(text);
        int cw = tw + (icon != null ? 16 + 6 : 0);
        int x = (w - cw) / 2;
        if (icon != null) {
            icon.paint(g, x, (h - 16) / 2f, 16, fg);
            x += 16 + 6;
        }
        g.setColor(fg);
        g.drawString(text, x, (h - fm.getHeight()) / 2 + fm.getAscent());
        g.dispose();
    }

    /** What is being installed right now, for a tooltip. */
    public String workingOn() {
        return working;
    }
}
