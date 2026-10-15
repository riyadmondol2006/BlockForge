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
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.geom.Arc2D;

import javax.swing.JComponent;
import javax.swing.Timer;

/** Indeterminate progress spinner. It only animates while it is on screen. */
public class Spinner extends JComponent {
    private static final long serialVersionUID = 1L;

    private final int size;
    private final Timer timer;
    private int angle;

    public Spinner(int size) {
        this.size = size;
        setOpaque(false);
        // a slower frame rate in light mode keeps the spinner from using a slow processor
        timer = new Timer(com.blockforge.launcher.utils.LightMode.frameDelayMs(), e -> {
            angle = (angle + Math.max(8, ((Timer) e.getSource()).getDelay() / 2)) % 360;
            repaint();
        });
    }

    @Override
    public void addNotify() {
        super.addNotify();
        timer.start();
    }

    @Override
    public void removeNotify() {
        timer.stop();
        super.removeNotify();
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(size, size);
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        float stroke = Math.max(3f, size / 10f);
        g.setStroke(new BasicStroke(stroke, BasicStroke.CAP_ROUND, BasicStroke.JOIN_ROUND));
        float d = size - stroke;
        float x = (getWidth() - d) / 2f;
        float y = (getHeight() - d) / 2f;
        g.setColor(Palette.outline());
        g.draw(new Arc2D.Float(x, y, d, d, 0, 360, Arc2D.OPEN));
        g.setColor(Palette.accent());
        g.draw(new Arc2D.Float(x, y, d, d, -angle, 100, Arc2D.OPEN));
        g.dispose();
    }
}
