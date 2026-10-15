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

import java.awt.AWTEvent;
import java.awt.AlphaComposite;
import java.awt.BorderLayout;
import java.awt.Color;
import java.awt.Component;
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.KeyEventDispatcher;
import java.awt.KeyboardFocusManager;
import java.awt.Point;
import java.awt.Rectangle;
import java.awt.RenderingHints;
import java.awt.Toolkit;
import java.awt.Window;
import java.awt.event.AWTEventListener;
import java.awt.event.ComponentAdapter;
import java.awt.event.ComponentEvent;
import java.awt.event.HierarchyEvent;
import java.awt.event.HierarchyListener;
import java.awt.event.KeyEvent;
import java.awt.event.MouseEvent;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.awt.geom.RoundRectangle2D;
import java.util.function.Consumer;

import javax.swing.BorderFactory;
import javax.swing.JComponent;
import javax.swing.JLayeredPane;
import javax.swing.JPanel;
import javax.swing.JRootPane;
import javax.swing.JScrollPane;
import javax.swing.SwingUtilities;
import javax.swing.Timer;

import com.blockforge.launcher.utils.LightMode;

/**
 * A panel that drops down from a button, drawn inside the launcher window like the rest of the interface: rounded,
 * with a soft shadow, and always within the window. It closes on a click outside it, Escape, scrolling the page or
 * leaving the window. Only one is open at a time.
 */
public final class Popover extends JPanel {
    private static final long serialVersionUID = 1L;

    private static final int SHADOW = 16;
    private static final int ARC = 20;
    /** space kept between the popover and the edges of the window */
    private static final int MARGIN = 8;
    /** space between the button and the popover */
    private static final int GAP = 6;
    private static final int OPEN_MS = 170;

    private static Popover open;
    /** the button whose press closed the last popover, so that same click doesn't open it again */
    private static Component closedByPressOn;
    private static long closedAt;

    private final Component anchor;
    private final JComponent content;
    private final JScrollPane scroll;
    private final JLayeredPane layers;
    private final Window window;
    /** where the popover wants its top left corner, in the coordinates of the layered pane */
    private final Point wanted;
    /** the height of what it drops from; it flips above that when there is no room below */
    private final int anchorHeight;
    private boolean upward;
    private float reveal = 1f;
    private Timer animation;
    private Consumer<KeyEvent> keyHandler;

    private final AWTEventListener outside = this::outsideEvent;
    private final KeyEventDispatcher keys = this::keyEvent;
    private final ComponentAdapter resized = new ComponentAdapter() {
        @Override
        public void componentResized(ComponentEvent e) {
            close();
        }
    };
    private final WindowAdapter focus = new WindowAdapter() {
        @Override
        public void windowLostFocus(WindowEvent e) {
            close();
        }
    };
    private final HierarchyListener anchorGone = e -> {
        if ((e.getChangeFlags() & HierarchyEvent.SHOWING_CHANGED) != 0 && !e.getComponent().isShowing()) {
            close();
        }
    };

    private Popover(Component anchor, JComponent content, JRootPane root, Point wanted, int anchorHeight) {
        super(new BorderLayout());
        this.anchor = anchor;
        this.content = content;
        this.layers = root.getLayeredPane();
        this.window = SwingUtilities.getWindowAncestor(root);
        this.wanted = wanted;
        this.anchorHeight = anchorHeight;
        setOpaque(false);
        setBorder(BorderFactory.createEmptyBorder(SHADOW, SHADOW, SHADOW, SHADOW));

        scroll = new JScrollPane(content, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
            JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        scroll.setBorder(null);
        scroll.setViewportBorder(null);
        scroll.setOpaque(false);
        scroll.getViewport().setOpaque(false);
        scroll.getVerticalScrollBar().setOpaque(false);
        scroll.getVerticalScrollBar().setUnitIncrement(24);
        add(scroll, BorderLayout.CENTER);
    }

    /** Drops the content down from under a button, lined up with its right edge, or above it near the bottom. */
    public static Popover below(Component button, JComponent content) {
        JRootPane root = SwingUtilities.getRootPane(button);
        if (root == null) {
            return null;
        }
        Point corner = SwingUtilities.convertPoint(button, button.getWidth(), button.getHeight() + GAP,
            root.getLayeredPane());
        corner.x -= content.getPreferredSize().width;
        return show(button, content, root, corner, button.getHeight() + GAP * 2);
    }

    /** Opens the content at a point of a component, like where it was right clicked. */
    public static Popover at(Component invoker, int x, int y, JComponent content) {
        JRootPane root = SwingUtilities.getRootPane(invoker);
        if (root == null) {
            return null;
        }
        return show(invoker, content, root, SwingUtilities.convertPoint(invoker, x, y, root.getLayeredPane()), 0);
    }

    private static Popover show(Component anchor, JComponent content, JRootPane root, Point wanted,
        int anchorHeight) {
        boolean sameClick = closedByPressOn == anchor && System.currentTimeMillis() - closedAt < 600;
        closedByPressOn = null;
        closeOpen();
        if (sameClick) {
            // the press on the button already closed this popover; the click that follows shouldn't reopen it
            return null;
        }

        Popover popover = new Popover(anchor, content, root, wanted, anchorHeight);
        popover.place(true);
        popover.layers.add(popover, JLayeredPane.POPUP_LAYER);
        popover.listen(true);
        open = popover;
        popover.animateOpen();
        return popover;
    }

    /** Closes the popover that is open, if there is one. */
    public static void closeOpen() {
        if (open != null) {
            open.close();
        }
    }

    public static boolean isOpen() {
        return open != null;
    }

    /** Gets key presses (other than Escape, which closes) while open, to move through the content. */
    public void onKey(Consumer<KeyEvent> handler) {
        this.keyHandler = handler;
    }

    /** Sizes and places the popover again after its content grew or shrank, keeping it on the same side. */
    public void relayout() {
        content.revalidate();
        place(false);
        layers.repaint();
    }

    private void place(boolean first) {
        Dimension pref = content.getPreferredSize();
        Rectangle area = new Rectangle(MARGIN, MARGIN, layers.getWidth() - MARGIN * 2,
            layers.getHeight() - MARGIN * 2);
        int height = Math.min(pref.height, area.height);
        boolean scrolls = height < pref.height;
        int width = pref.width + (scrolls ? scroll.getVerticalScrollBar().getPreferredSize().width : 0);

        Rectangle card;
        if (first) {
            Point corner = Popups.place(wanted, anchorHeight, new Dimension(width, height), area);
            upward = corner.y < wanted.y;
            card = new Rectangle(corner.x, corner.y, width, height);
        } else {
            // grow away from the button: keep the edge next to it where it is
            card = new Rectangle(getX() + SHADOW, getY() + SHADOW, width, getHeight() - SHADOW * 2);
            if (upward) {
                int bottom = card.y + card.height;
                card.y = Math.max(area.y, bottom - height);
                card.height = bottom - card.y;
            } else {
                card.height = Math.min(height, area.y + area.height - card.y);
            }
        }

        scroll.setPreferredSize(new Dimension(card.width, card.height));
        setBounds(card.x - SHADOW, card.y - SHADOW, card.width + SHADOW * 2, card.height + SHADOW * 2);
        validate();
    }

    private void animateOpen() {
        if (LightMode.isOn()) {
            return;
        }
        long start = System.currentTimeMillis();
        reveal = 0f;
        animation = new Timer(LightMode.frameDelayMs(), e -> {
            reveal = Math.min(1f, (System.currentTimeMillis() - start) / (float) OPEN_MS);
            repaint();
            if (reveal >= 1f) {
                ((Timer) e.getSource()).stop();
            }
        });
        animation.start();
    }

    private void listen(boolean on) {
        if (on) {
            Toolkit.getDefaultToolkit().addAWTEventListener(outside,
                AWTEvent.MOUSE_EVENT_MASK | AWTEvent.MOUSE_WHEEL_EVENT_MASK);
            KeyboardFocusManager.getCurrentKeyboardFocusManager().addKeyEventDispatcher(keys);
            layers.addComponentListener(resized);
            anchor.addHierarchyListener(anchorGone);
            if (window != null) {
                window.addWindowFocusListener(focus);
            }
        } else {
            Toolkit.getDefaultToolkit().removeAWTEventListener(outside);
            KeyboardFocusManager.getCurrentKeyboardFocusManager().removeKeyEventDispatcher(keys);
            layers.removeComponentListener(resized);
            anchor.removeHierarchyListener(anchorGone);
            if (window != null) {
                window.removeWindowFocusListener(focus);
            }
        }
    }

    private void close() {
        if (open != this) {
            return;
        }
        open = null;
        listen(false);
        if (animation != null) {
            animation.stop();
        }
        Rectangle bounds = getBounds();
        layers.remove(this);
        layers.repaint(bounds);
    }

    private void outsideEvent(AWTEvent event) {
        if (!(event instanceof MouseEvent)) {
            return;
        }
        MouseEvent e = (MouseEvent) event;
        if (e.getID() != MouseEvent.MOUSE_PRESSED && e.getID() != MouseEvent.MOUSE_WHEEL) {
            return;
        }
        Component source = e.getComponent();
        if (source != null && SwingUtilities.isDescendingFrom(source, this)) {
            return;
        }
        if (e.getID() == MouseEvent.MOUSE_PRESSED && source != null
            && SwingUtilities.isDescendingFrom(source, anchor)) {
            closedByPressOn = anchor;
            closedAt = System.currentTimeMillis();
        }
        close();
    }

    private boolean keyEvent(KeyEvent e) {
        if (e.getID() != KeyEvent.KEY_PRESSED || (window != null && !window.isFocused())) {
            return false;
        }
        if (e.getKeyCode() == KeyEvent.VK_ESCAPE) {
            close();
            return true;
        }
        if (keyHandler != null) {
            keyHandler.accept(e);
            return e.isConsumed();
        }
        return false;
    }

    /** The colour of the popover: a step lighter than the cards in the dark theme, so it stands off them. */
    static Color surface() {
        return Palette.dark() ? Palette.mix(Palette.card(), Palette.text(), 0.05f) : Palette.card();
    }

    @Override
    protected void paintComponent(Graphics g0) {
        Graphics2D g = (Graphics2D) g0.create();
        g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
        float w = getWidth() - SHADOW * 2;
        float h = getHeight() - SHADOW * 2;

        // a soft shadow, a little lower than the card, as if it sits above the page
        int strongest = Palette.dark() ? 22 : 9;
        for (int i = SHADOW; i > 0; i -= 2) {
            float t = 1f - i / (float) SHADOW;
            g.setColor(new Color(0, 0, 0, Math.round(strongest * t * t)));
            g.fill(new RoundRectangle2D.Float(SHADOW - i, SHADOW - i + 4, w + i * 2, h + i * 2, ARC + i * 2,
                ARC + i * 2));
        }

        g.setColor(surface());
        g.fill(new RoundRectangle2D.Float(SHADOW, SHADOW, w, h, ARC, ARC));
        g.setColor(Palette.outline());
        g.draw(new RoundRectangle2D.Float(SHADOW + 0.5f, SHADOW + 0.5f, w - 1, h - 1, ARC, ARC));
        g.dispose();
    }

    @Override
    public void paint(Graphics g0) {
        if (reveal >= 1f) {
            super.paint(g0);
            return;
        }

        // unfold from the button: grow from part of the height while fading in and sliding the last few pixels
        float eased = 1f - (1f - reveal) * (1f - reveal) * (1f - reveal);
        Graphics2D g = (Graphics2D) g0.create();
        g.setComposite(AlphaComposite.SrcOver.derive(Math.min(1f, eased * 1.5f)));
        int h = getHeight();
        int shown = Math.round(h * (0.4f + 0.6f * eased));
        if (upward) {
            g.clipRect(0, h - shown, getWidth(), shown);
        } else {
            g.clipRect(0, 0, getWidth(), shown);
        }
        g.translate(0, Math.round((1f - eased) * 8) * (upward ? 1 : -1));
        super.paint(g);
        g.dispose();
    }
}
