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
import java.awt.Container;
import java.awt.Dimension;
import java.awt.Insets;
import java.awt.LayoutManager;
import java.awt.Rectangle;
import java.util.ArrayList;
import java.util.List;

import javax.swing.JComponent;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.Scrollable;
import javax.swing.SwingConstants;

import com.blockforge.launcher.gui.layouts.WrapLayout;

/**
 * A panel that always matches the width of the scroll pane it's in. With a gap it lays its children out as a grid of
 * cards: as many columns as fit at the cards' preferred width, then every card is stretched by the same amount so each
 * row fills the whole width and its last card lines up with the right edge of whatever sits above the grid.
 */
public class FitWidthPanel extends JPanel implements Scrollable {
    private static final long serialVersionUID = 1L;

    public FitWidthPanel(int gap) {
        super(gap == 0 ? new WrapLayout(WrapLayout.LEFT, 0, 0) : new CardGrid(gap));
        setOpaque(false);
    }

    /** Columns that fit in the width at the minimum card width, never fewer than one. */
    static int columns(int width, int minWidth, int gap) {
        return Math.max(1, (width + gap) / (Math.max(1, minWidth) + gap));
    }

    /** Lays cards out in equal columns that together fill the width exactly. */
    static final class CardGrid implements LayoutManager {
        private final int gap;

        CardGrid(int gap) {
            this.gap = gap;
        }

        @Override
        public void addLayoutComponent(String name, Component comp) {
        }

        @Override
        public void removeLayoutComponent(Component comp) {
        }

        private static List<Component> visible(Container target) {
            List<Component> shown = new ArrayList<>();
            for (Component c : target.getComponents()) {
                if (c.isVisible()) {
                    shown.add(c);
                }
            }
            return shown;
        }

        /** The narrowest a card may get: the widest preferred width among them. */
        private static int minCardWidth(List<Component> shown) {
            int min = 0;
            for (Component c : shown) {
                min = Math.max(min, c.getPreferredSize().width);
            }
            return min;
        }

        /** The width to fill: the panel's own once laid out, before that the nearest ancestor with a size. */
        private static int availableWidth(Container target) {
            Container c = target;
            while (c.getWidth() == 0 && c.getParent() != null) {
                c = c.getParent();
            }
            Insets in = target.getInsets();
            int width = c.getWidth() - in.left - in.right;
            return width <= 0 ? Integer.MAX_VALUE / 4 : width;
        }

        private static int[] rowHeights(List<Component> shown, int cols) {
            int[] heights = new int[(shown.size() + cols - 1) / cols];
            for (int i = 0; i < shown.size(); i++) {
                heights[i / cols] = Math.max(heights[i / cols], shown.get(i).getPreferredSize().height);
            }
            return heights;
        }

        @Override
        public Dimension preferredLayoutSize(Container target) {
            synchronized (target.getTreeLock()) {
                Insets in = target.getInsets();
                List<Component> shown = visible(target);
                int min = minCardWidth(shown);
                int[] rows = rowHeights(shown, columns(availableWidth(target), min, gap));
                int height = 0;
                for (int i = 0; i < rows.length; i++) {
                    height += rows[i] + (i > 0 ? gap : 0);
                }
                // asks for one card's width only, so the grid never pushes its container wider
                return new Dimension(min + in.left + in.right, height + in.top + in.bottom);
            }
        }

        @Override
        public Dimension minimumLayoutSize(Container target) {
            return new Dimension(0, preferredLayoutSize(target).height);
        }

        @Override
        public void layoutContainer(Container target) {
            synchronized (target.getTreeLock()) {
                Insets in = target.getInsets();
                int width = target.getWidth() - in.left - in.right;
                List<Component> shown = visible(target);
                int cols = columns(width, minCardWidth(shown), gap);
                int[] rows = rowHeights(shown, cols);
                int cell = Math.max(0, (width - (cols - 1) * gap) / cols);
                int extra = Math.max(0, width - (cols - 1) * gap - cell * cols);
                int y = in.top;
                for (int r = 0; r < rows.length; r++) {
                    int x = in.left;
                    for (int c = 0; c < cols; c++) {
                        // the pixels that don't divide evenly go one each to the first cards of the row
                        int w = cell + (c < extra ? 1 : 0);
                        int i = r * cols + c;
                        if (i < shown.size()) {
                            shown.get(i).setBounds(x, y, w, rows[r]);
                        }
                        x += w + gap;
                    }
                    y += rows[r] + gap;
                }
            }
        }
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

    /**
     * Keeps a page header that sits above {@code scroll} lined up with the scrolled content: while the scroll bar
     * shows, it takes width from the content, so the header's right padding grows by the same amount.
     */
    public static void alignHeader(JComponent header, JScrollPane scroll, int top, int side, int bottom) {
        javax.swing.JScrollBar bar = scroll.getVerticalScrollBar();
        Runnable update = () -> {
            int extra = bar.isVisible() ? bar.getWidth() : 0;
            java.awt.Insets now = header.getInsets();
            if (now.right != side + extra) {
                header.setBorder(javax.swing.BorderFactory.createEmptyBorder(top, side, bottom, side + extra));
                header.revalidate();
            }
        };
        bar.addComponentListener(new java.awt.event.ComponentAdapter() {
            @Override
            public void componentShown(java.awt.event.ComponentEvent e) {
                update.run();
            }

            @Override
            public void componentHidden(java.awt.event.ComponentEvent e) {
                update.run();
            }

            @Override
            public void componentResized(java.awt.event.ComponentEvent e) {
                update.run();
            }
        });
        header.setBorder(javax.swing.BorderFactory.createEmptyBorder(top, side, bottom, side));
    }

    /** A FlowLayout row of buttons aligned right, without the gap FlowLayout leaves after the last one. */
    public static JPanel rightButtons() {
        JPanel row = new JPanel(new java.awt.FlowLayout(java.awt.FlowLayout.RIGHT, 10, 0));
        row.setOpaque(false);
        row.setBorder(javax.swing.BorderFactory.createEmptyBorder(0, 0, 0, -10));
        return row;
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
        // no track strip of its own: the page shows through, in the light theme too
        pane.getVerticalScrollBar().setOpaque(false);
        pane.setWheelScrollingEnabled(true);
        return pane;
    }
}
