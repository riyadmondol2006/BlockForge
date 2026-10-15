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
import java.awt.Rectangle;

import javax.swing.JComponent;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.Scrollable;
import javax.swing.SwingConstants;

import com.blockforge.launcher.gui.layouts.WrapLayout;

/** A panel that wraps its children like text and always matches the width of the scroll pane it's in. */
public class FitWidthPanel extends JPanel implements Scrollable {
    private static final long serialVersionUID = 1L;

    private final int gap;

    public FitWidthPanel(int gap) {
        super(new WrapLayout(WrapLayout.LEFT, 0, 0));
        this.gap = gap;
        setOpaque(false);
    }

    /** Adds a card; every card gets room to its right and below it so rows and columns are evenly spaced. */
    @Override
    public java.awt.Component add(java.awt.Component comp) {
        if (gap == 0) {
            return super.add(comp);
        }
        JPanel cell = new JPanel(new java.awt.BorderLayout());
        cell.setOpaque(false);
        cell.setBorder(javax.swing.BorderFactory.createEmptyBorder(0, 0, gap, gap));
        cell.add(comp, java.awt.BorderLayout.CENTER);
        super.add(cell);
        return comp;
    }

    @Override
    public Dimension getPreferredScrollableViewportSize() {
        return getPreferredSize();
    }

    @Override
    public int getScrollableUnitIncrement(Rectangle visibleRect, int orientation, int direction) {
        return 32;
    }

    @Override
    public int getScrollableBlockIncrement(Rectangle visibleRect, int orientation, int direction) {
        return orientation == SwingConstants.VERTICAL ? visibleRect.height - 48 : visibleRect.width - 48;
    }

    @Override
    public boolean getScrollableTracksViewportWidth() {
        return true;
    }

    @Override
    public boolean getScrollableTracksViewportHeight() {
        return false;
    }

    /** Wraps any component in a scroll pane without borders that scrolls vertically only. */
    public static JScrollPane scroll(JComponent content) {
        JScrollPane pane = new JScrollPane(content, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
            JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        pane.setBorder(null);
        pane.setViewportBorder(null);
        pane.setOpaque(false);
        pane.getViewport().setOpaque(false);
        pane.getVerticalScrollBar().setUnitIncrement(32);
        pane.setWheelScrollingEnabled(true);
        return pane;
    }
}
