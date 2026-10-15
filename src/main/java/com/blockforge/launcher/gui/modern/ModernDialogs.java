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

import java.awt.BorderLayout;
import java.awt.Color;
import java.awt.Component;
import java.awt.Container;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Insets;
import java.awt.LayoutManager;
import java.awt.Window;
import java.awt.event.HierarchyEvent;
import java.awt.event.HierarchyListener;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.Icon;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JOptionPane;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JSpinner;
import javax.swing.RootPaneContainer;
import javax.swing.SwingUtilities;
import javax.swing.border.Border;

import com.blockforge.launcher.utils.Utils;

/**
 * Building blocks shared by every BlockForge dialog: a page coloured surface with comfortable padding, a header with
 * a title and an optional subtitle, and a footer with the buttons on the right. Colours are read from {@link Palette}
 * when painting so dialogs follow the light or dark theme.
 */
public final class ModernDialogs {
    /** padding around the content of a dialog */
    public static final int PAD = 24;

    /** gaps for label/field rows laid out with a GridBagLayout */
    public static final Insets LABEL_INSETS = new Insets(7, 0, 7, 16);
    public static final Insets FIELD_INSETS = new Insets(7, 0, 7, 0);

    private ModernDialogs() {
    }

    /** A panel painted with the window background colour. */
    public static class Surface extends JPanel {
        private static final long serialVersionUID = 1L;

        public Surface(LayoutManager layout) {
            super(layout);
            setOpaque(true);
        }

        @Override
        public Color getBackground() {
            return Palette.bg();
        }
    }

    /** A rounded card with an outline, used to group content inside a dialog. */
    public static RoundPanel card(LayoutManager layout) {
        RoundPanel card = new RoundPanel(layout, 20, Palette::card);
        card.line(Palette::outline);
        card.pad(14, 16, 14, 16);
        return card;
    }

    /**
     * A heading with a rounded card under it holding a scroll pane, used for the lists of mods. The scroll pane
     * loses its own border so the card outline is the only frame.
     */
    public static JPanel listCard(JComponent heading, JScrollPane scroller, JComponent shown) {
        scroller.setBorder(BorderFactory.createEmptyBorder());
        scroller.getVerticalScrollBar().setOpaque(false);
        scroller.setOpaque(false);

        JPanel column = new JPanel(new BorderLayout());
        column.setOpaque(false);
        heading.setBorder(BorderFactory.createEmptyBorder(0, 4, 8, 0));
        column.add(heading, BorderLayout.NORTH);

        RoundPanel card = card(new BorderLayout());
        card.pad(6, 6, 6, 2);
        card.add(shown, BorderLayout.CENTER);
        column.add(card, BorderLayout.CENTER);

        return column;
    }

    /** Title in large bold type with an optional dimmed line under it. */
    public static JPanel header(String title, String subtitle) {
        return header(title, subtitle, null);
    }

    /** Title, optional subtitle and an optional icon shown in a soft accent circle on the left. */
    public static JPanel header(String title, String subtitle, Icons icon) {
        JPanel header = new JPanel(new BorderLayout(14, 0));
        header.setOpaque(false);
        header.setBorder(BorderFactory.createEmptyBorder(0, 0, 18, 0));

        if (icon != null) {
            JLabel badge = new JLabel() {
                private static final long serialVersionUID = 1L;

                @Override
                protected void paintComponent(java.awt.Graphics g0) {
                    java.awt.Graphics2D g = (java.awt.Graphics2D) g0.create();
                    g.setRenderingHint(java.awt.RenderingHints.KEY_ANTIALIASING,
                        java.awt.RenderingHints.VALUE_ANTIALIAS_ON);
                    g.setColor(Palette.accentSoft());
                    g.fillOval(0, 0, 44, 44);
                    icon.paint(g, 11, 11, 22, Palette.accent());
                    g.dispose();
                }
            };
            badge.setPreferredSize(new Dimension(44, 44));
            JPanel holder = new JPanel(new BorderLayout());
            holder.setOpaque(false);
            holder.add(badge, BorderLayout.NORTH);
            header.add(holder, BorderLayout.WEST);
        }

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel t = Lbl.text(title, 20f, true, Palette::text);
        t.setName("dialogTitle");
        t.setAlignmentX(Component.LEFT_ALIGNMENT);
        text.add(t);
        if (subtitle != null && !subtitle.isEmpty()) {
            text.add(Box.createVerticalStrut(3));
            JLabel s = Lbl.text(subtitle, 13f, false, Palette::textDim);
            s.setName("dialogSubtitle");
            s.setAlignmentX(Component.LEFT_ALIGNMENT);
            text.add(s);
        }
        header.add(text, BorderLayout.CENTER);
        return header;
    }

    /** Buttons aligned to the right, the first one given is the leftmost. */
    public static JPanel footer(Component... buttons) {
        return footerWithLeft(null, buttons);
    }

    /** Buttons aligned to the right with an optional component (a checkbox, a status) on the left. */
    public static JPanel footerWithLeft(Component left, Component... buttons) {
        JPanel footer = new JPanel(new BorderLayout(12, 0));
        footer.setOpaque(false);
        footer.setBorder(BorderFactory.createEmptyBorder(18, 0, 0, 0));

        JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        right.setOpaque(false);
        for (Component b : buttons) {
            if (b != null) {
                right.add(b);
            }
        }
        footer.add(right, BorderLayout.EAST);

        if (left != null) {
            JPanel holder = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
            holder.setOpaque(false);
            holder.add(left);
            footer.add(holder, BorderLayout.CENTER);
        }
        return footer;
    }

    /** The main action of a dialog. */
    public static Btn primary(String text) {
        return new Btn(text, Btn.Kind.FILLED);
    }

    /** Secondary actions such as Cancel. */
    public static Btn secondary(String text) {
        return new Btn(text, Btn.Kind.TONAL);
    }

    /** Destructive actions. */
    public static Btn danger(String text) {
        return new Btn(text, Btn.Kind.DANGER);
    }

    /** A form label in the body text colour. */
    public static JLabel label(String text) {
        return Lbl.text(text, 13f, false, Palette::text);
    }

    /**
     * Replaces the content of a dialog with a padded surface holding the header, body and footer. Any of them can
     * be null.
     */
    public static Surface install(JDialog dialog, JComponent header, JComponent body, JComponent footer) {
        Surface root = surface(header, body, footer);
        dialog.setContentPane(root);
        dialog.setIconImage(Utils.getImage("/assets/image/icon.png"));
        return root;
    }

    /**
     * For dialogs sized with pack(): window managers only report the size of the title bar once a window is on
     * screen, so a packed dialog can come out a little short and squash its form. This grows the window by the
     * missing amount the first time it is shown.
     */
    public static void growToFitOnShow(Window window) {
        if (!(window instanceof RootPaneContainer)) {
            return;
        }
        Container content = ((RootPaneContainer) window).getContentPane();
        content.addHierarchyListener(new HierarchyListener() {
            @Override
            public void hierarchyChanged(HierarchyEvent e) {
                if ((e.getChangeFlags() & HierarchyEvent.SHOWING_CHANGED) == 0 || !content.isShowing()) {
                    return;
                }
                content.removeHierarchyListener(this);
                // the window manager can report the title bar a moment after the window appears, so look a few
                // times during the first second
                int[] checks = { 0 };
                javax.swing.Timer timer = new javax.swing.Timer(250, null);
                timer.addActionListener(ev -> {
                    if (++checks[0] >= 4 || !window.isShowing()) {
                        timer.stop();
                    }
                    Dimension want = content.getPreferredSize();
                    Dimension have = content.getSize();
                    int dx = Math.min(120, Math.max(0, want.width - have.width));
                    int dy = Math.min(120, Math.max(0, want.height - have.height));
                    if (dx > 0 || dy > 0) {
                        window.setSize(window.getWidth() + dx, window.getHeight() + dy);
                        window.validate();
                    }
                });
                timer.setInitialDelay(50);
                timer.start();
            }
        });
    }

    /** Same as {@link #install(JDialog, JComponent, JComponent, JComponent)} for frames. */
    public static Surface install(JFrame frame, JComponent header, JComponent body, JComponent footer) {
        Surface root = surface(header, body, footer);
        frame.setContentPane(root);
        return root;
    }

    private static Surface surface(JComponent header, JComponent body, JComponent footer) {
        Surface root = new Surface(new BorderLayout());
        root.setBorder(BorderFactory.createEmptyBorder(PAD, PAD + 4, PAD - 4, PAD + 4));
        if (header != null) {
            root.add(header, BorderLayout.NORTH);
        }
        if (body != null) {
            root.add(body, BorderLayout.CENTER);
        }
        if (footer != null) {
            root.add(footer, BorderLayout.SOUTH);
        }
        return root;
    }

    /**
     * Makes plain panels inside the given container see-through so the dialog surface shows behind them. Panels with
     * their own purpose (scroll pane views, lists, cards) are left alone.
     */
    public static <T extends Component> T blend(T component) {
        if (component instanceof JPanel && isPlainPanel(component)) {
            ((JPanel) component).setOpaque(false);
        }
        if (component instanceof Container && !(component instanceof JScrollPane)
            && !(component instanceof JComboBox) && !(component instanceof JSpinner)) {
            for (Component child : ((Container) component).getComponents()) {
                blend(child);
            }
        }
        return component;
    }

    /** Makes a component and its plain child panels see-through. */
    public static <T extends JComponent> T transparent(T component) {
        component.setOpaque(false);
        return blend(component);
    }

    private static boolean isPlainPanel(Component c) {
        Class<?> type = c.getClass();
        return type == JPanel.class || type == Box.class
            || (type.isAnonymousClass() && type.getSuperclass() == JPanel.class);
    }

    /** A scroll pane without a border whose viewport shows the dialog surface. */
    public static JScrollPane scroll(JComponent view) {
        JScrollPane pane = new JScrollPane(view, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED,
            JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        pane.setBorder(BorderFactory.createEmptyBorder());
        pane.setViewportBorder(null);
        pane.setOpaque(false);
        pane.getViewport().setOpaque(false);
        pane.getVerticalScrollBar().setUnitIncrement(24);
        return pane;
    }

    /** A thin rounded outline for scroll panes and lists that sit directly on the surface. */
    public static Border outline() {
        return new javax.swing.border.AbstractBorder() {
            private static final long serialVersionUID = 1L;

            @Override
            public void paintBorder(Component c, java.awt.Graphics g0, int x, int y, int width, int height) {
                java.awt.Graphics2D g = (java.awt.Graphics2D) g0.create();
                g.setRenderingHint(java.awt.RenderingHints.KEY_ANTIALIASING,
                    java.awt.RenderingHints.VALUE_ANTIALIAS_ON);
                g.setColor(Palette.outline());
                g.draw(new java.awt.geom.RoundRectangle2D.Float(x + 0.5f, y + 0.5f, width - 1, height - 1, 14, 14));
                g.dispose();
            }

            @Override
            public Insets getBorderInsets(Component c) {
                return new Insets(4, 4, 4, 4);
            }
        };
    }

    /**
     * Sets the preferred width of an input while keeping the height the theme gives it, so every input in a dialog
     * has the same height.
     */
    public static void preferWidth(JComponent input, int width) {
        input.setPreferredSize(null);
        input.setPreferredSize(new Dimension(width, input.getPreferredSize().height));
    }

    /** The icon shown next to the message of a {@link JOptionPane} of the given type. */
    public static Icon messageIcon(int type) {
        switch (type) {
            case JOptionPane.ERROR_MESSAGE:
                return Icons.WARNING.asIcon(30, Palette.danger());
            case JOptionPane.WARNING_MESSAGE:
                return Icons.WARNING.asIcon(30, Palette.warn());
            case JOptionPane.INFORMATION_MESSAGE:
            case JOptionPane.QUESTION_MESSAGE:
                return Icons.INFO.asIcon(30, Palette.accent());
            default:
                return null;
        }
    }

    /**
     * Restyles the dialog created by a {@link JOptionPane}: the page surface, generous padding and see-through inner
     * panels. Nothing about how the option pane behaves is changed.
     */
    public static void polish(JOptionPane pane, Window dialog) {
        pane.setBackground(Palette.bg());
        pane.setOpaque(true);
        pane.setBorder(BorderFactory.createEmptyBorder(PAD, PAD + 4, PAD - 4, PAD + 4));
        for (Component child : pane.getComponents()) {
            blend(child);
        }
        if (dialog instanceof JDialog) {
            ((JDialog) dialog).getContentPane().setBackground(Palette.bg());
        }
        dialog.pack();
        dialog.setLocationRelativeTo(dialog.getOwner());
    }
}
