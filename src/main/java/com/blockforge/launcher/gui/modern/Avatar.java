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
import java.awt.Image;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;
import java.awt.image.BufferedImage;
import java.io.File;

import javax.imageio.ImageIO;
import javax.swing.ImageIcon;
import javax.swing.JComponent;

import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.utils.Utils;

/** A player's head as a rounded square. */
public class Avatar extends JComponent {
    private static final long serialVersionUID = 1L;

    private final int size;
    private MicrosoftAccount account;

    public Avatar(int size) {
        this.size = size;
        setOpaque(false);
    }

    /**
     * The account's face. Painting used to read and decode the skin file (twice) on every repaint, on the UI thread;
     * now it is only read again when the skin file changes, for example after "Reload skin".
     */
    private Image head;
    private long headStamp = Long.MIN_VALUE;

    public void setAccount(MicrosoftAccount account) {
        this.account = account;
        this.head = null;
        this.headStamp = Long.MIN_VALUE;
        repaint();
    }

    private File skinFile() {
        return FileSystem.SKINS.resolve((account.uuid == null ? "default" : account.getUUIDNoDashes()) + ".png")
            .toFile();
    }

    private Image head() {
        long stamp = skinFile().lastModified();
        if (head == null || stamp != headStamp) {
            headStamp = stamp;
            head = face(account, skinFile());
        }
        return head;
    }

    /**
     * The 8x8 face (and hat layer) straight from the skin file, so it can be scaled up with sharp pixels. Falls back
     * to the launcher's usual head icon if the skin can't be read.
     */
    private static Image face(MicrosoftAccount account, File file) {
        try {
            BufferedImage skin = file.isFile() ? ImageIO.read(file) : null;
            if (skin != null && skin.getWidth() >= 48 && skin.getHeight() >= 16) {
                BufferedImage face = new BufferedImage(8, 8, BufferedImage.TYPE_INT_ARGB);
                Graphics2D g = face.createGraphics();
                g.drawImage(skin.getSubimage(8, 8, 8, 8), 0, 0, null);
                BufferedImage hat = skin.getSubimage(40, 8, 8, 8);
                // same rule as SkinUtils.getHead: a solid hat layer is usually junk from old skin editors
                if (Utils.nonTransparentPixels(hat) <= 32) {
                    g.drawImage(hat, 0, 0, null);
                }
                g.dispose();
                return face;
            }
        } catch (Exception ignored) {
            // an unreadable skin just shows the default head
        }

        ImageIcon icon = account.getMinecraftHead();
        return icon == null ? null : icon.getImage();
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
            Image face = head();
            if (face != null) {
                // clip() narrows the clip it was given; setClip() would throw it away and paint over whatever
                // covers this component, such as the edge of a scroll pane
                g.clip(new RoundRectangle2D.Float(0, 0, size, size, arc, arc));
                g.drawImage(face, 0, 0, size, size, null);
            }
        } else {
            Icons.PERSON.paint(g, size * 0.25f, size * 0.25f, size * 0.5f, Palette.textDim());
        }
        g.dispose();
    }
}
