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
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;

import javax.swing.ImageIcon;
import javax.swing.JComponent;

import com.blockforge.launcher.data.MicrosoftAccount;

/** A player's head as a rounded square. */
public class Avatar extends JComponent {
    private static final long serialVersionUID = 1L;

    private final int size;
    private MicrosoftAccount account;

    public Avatar(int size) {
        this.size = size;
        setOpaque(false);
    }

    public void setAccount(MicrosoftAccount account) {
        this.account = account;
        repaint();
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(size, size);
    }

    @Override
    public Dimension getMinimumSize() {
        return getPreferredSize();
    }

    @Override
    public Dimension getMaximumSize() {
        return getPreferredSize();
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
        float arc = size * 0.28f;
        g.setColor(Palette.field());
        g.fill(new RoundRectangle2D.Float(0, 0, size, size, arc, arc));
        if (account != null) {
            ImageIcon head = account.getMinecraftHead();
            if (head != null) {
                g.setClip(new RoundRectangle2D.Float(0, 0, size, size, arc, arc));
                g.drawImage(head.getImage(), 0, 0, size, size, null);
            }
        } else {
            Icons.PERSON.paint(g, size * 0.25f, size * 0.25f, size * 0.5f, Palette.textDim());
        }
        g.dispose();
    }
}
