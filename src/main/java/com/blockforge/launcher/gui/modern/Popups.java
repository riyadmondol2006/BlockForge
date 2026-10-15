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

import java.awt.Component;
import java.awt.Dimension;
import java.awt.Point;
import java.awt.Rectangle;

import javax.swing.JPopupMenu;
import javax.swing.JRootPane;
import javax.swing.SwingUtilities;

/**
 * Opens popup menus inside the launcher window. Swing only keeps a menu on the screen, so one opened from a card near
 * the bottom of the window would hang over the taskbar and the desktop below it.
 */
public final class Popups {
    private Popups() {
    }

    /** Opens the menu under a button, or above it when there isn't room below. */
    public static void below(JPopupMenu menu, Component button) {
        show(menu, button, 0, button.getHeight(), button.getHeight());
    }

    /** Opens the menu at a point (like a right click), or above that point when there isn't room below. */
    public static void at(JPopupMenu menu, Component invoker, int x, int y) {
        show(menu, invoker, x, y, 0);
    }

    private static void show(JPopupMenu menu, Component invoker, int x, int y, int anchorHeight) {
        JRootPane root = SwingUtilities.getRootPane(invoker);
        if (root == null || !root.isShowing() || !invoker.isShowing()) {
            menu.show(invoker, x, y);
            return;
        }

        Point origin = invoker.getLocationOnScreen();
        Point rootOrigin = root.getLocationOnScreen();
        Rectangle area = new Rectangle(rootOrigin.x, rootOrigin.y, root.getWidth(), root.getHeight());
        Point placed = place(new Point(origin.x + x, origin.y + y), anchorHeight, menu.getPreferredSize(), area);
        menu.show(invoker, placed.x - origin.x, placed.y - origin.y);
    }

    /**
     * Where the top left corner of a menu of {@code size} goes, in the same coordinates as {@code area}. It opens at
     * {@code wanted}; if it doesn't fit below, it moves above the anchor (the {@code anchorHeight} pixels just above
     * {@code wanted}, like the button it came from), and in the end it is kept inside the area as much as it fits.
     */
    static Point place(Point wanted, int anchorHeight, Dimension size, Rectangle area) {
        int x = wanted.x;
        int y = wanted.y;

        if (y + size.height > area.y + area.height) {
            int above = wanted.y - anchorHeight - size.height;
            if (above >= area.y) {
                y = above;
            } else {
                // fits neither way, so use as much of the window as possible from the top
                y = Math.max(area.y, area.y + area.height - size.height);
            }
        }

        if (x + size.width > area.x + area.width) {
            x = area.x + area.width - size.width;
        }
        x = Math.max(area.x, x);
        return new Point(x, y);
    }
}
