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
import java.awt.Cursor;
import java.awt.Dimension;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.event.KeyEvent;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.RoundRectangle2D;
import java.util.HashMap;
import java.util.Map;
import java.util.function.IntConsumer;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JMenuItem;
import javax.swing.JPanel;
import javax.swing.JPopupMenu;
import javax.swing.KeyStroke;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.utils.Utils;

import io.reactivex.rxjava3.disposables.CompositeDisposable;

/**
 * The navigation rail on the left: where you are, where you can go and who you are playing as.
 */
public class Sidebar extends JPanel {
    private static final long serialVersionUID = 1L;

    public static final int WIDTH = 252;

    private final Map<Integer, NavItem> items = new HashMap<>();
    private final IntConsumer navigate;
    private final Avatar chipAvatar = new Avatar(38);
    private final JLabel chipName = Lbl.text("", 14f, true, Palette::text);
    private final JLabel chipKind = Lbl.text("", 12f, false, Palette::textDim);
    private final CompositeDisposable subscriptions = new CompositeDisposable();

    public Sidebar(IntConsumer navigate) {
        this.navigate = navigate;
        setOpaque(true);
        setLayout(new BorderLayout());
        setPreferredSize(new Dimension(WIDTH, 0));
        setBorder(BorderFactory.createEmptyBorder(22, 16, 16, 16));

        JPanel top = new JPanel();
        top.setOpaque(false);
        top.setLayout(new BoxLayout(top, BoxLayout.Y_AXIS));
        top.add(brand());
        top.add(Box.createVerticalStrut(26));
        top.add(add(UIConstants.LAUNCHER_NEWS_TAB, Icons.HOME, GetText.tr("Home")));
        top.add(Box.createVerticalStrut(4));
        top.add(add(UIConstants.LAUNCHER_INSTANCES_TAB, Icons.LIBRARY, GetText.tr("Library")));
        top.add(Box.createVerticalStrut(4));
        top.add(add(UIConstants.LAUNCHER_PACKS_TAB, Icons.EXPLORE, GetText.tr("Discover")));
        top.add(Box.createVerticalStrut(22));
        top.add(sectionLabel(GetText.tr("Build")));
        top.add(add(UIConstants.LAUNCHER_CREATE_PACK_TAB, Icons.CUBE, GetText.tr("Create")));
        top.add(Box.createVerticalStrut(4));
        top.add(add(UIConstants.LAUNCHER_TOOLS_TAB, Icons.TERMINAL, GetText.tr("Tools")));
        add(top, BorderLayout.NORTH);

        JPanel bottom = new JPanel();
        bottom.setOpaque(false);
        bottom.setLayout(new BoxLayout(bottom, BoxLayout.Y_AXIS));
        bottom.add(accountChip());
        bottom.add(Box.createVerticalStrut(12));
        bottom.add(add(UIConstants.LAUNCHER_SETTINGS_TAB, Icons.SETTINGS, GetText.tr("Settings")));
        add(bottom, BorderLayout.SOUTH);
    }

    @Override
    public Color getBackground() {
        return Palette.rail();
    }

    private JComponent brand() {
        JPanel brand = new JPanel(new BorderLayout(12, 0));
        brand.setOpaque(false);
        brand.setBorder(BorderFactory.createEmptyBorder(0, 6, 0, 0));
        brand.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
        brand.setAlignmentX(LEFT_ALIGNMENT);

        JLabel logo = new JLabel();
        logo.setPreferredSize(new Dimension(40, 40));
        java.awt.Image image = Utils.getImage("/assets/image/icon.png");
        if (image != null) {
            logo.setIcon(new javax.swing.ImageIcon(image.getScaledInstance(40, 40, java.awt.Image.SCALE_SMOOTH)));
        }
        brand.add(logo, BorderLayout.WEST);
        brand.add(Lbl.text(Constants.LAUNCHER_NAME, 19f, true, Palette::text), BorderLayout.CENTER);
        return brand;
    }

    private JComponent sectionLabel(String text) {
        JLabel l = Lbl.text(text.toUpperCase(java.util.Locale.ROOT), 11f, true, Palette::textDim);
        l.setBorder(BorderFactory.createEmptyBorder(0, 14, 6, 0));
        l.setAlignmentX(LEFT_ALIGNMENT);
        l.setMaximumSize(new Dimension(Integer.MAX_VALUE, 24));
        return l;
    }

    private NavItem add(int id, Icons icon, String label) {
        NavItem item = new NavItem(icon, label, () -> navigate.accept(id));
        items.put(id, item);
        return item;
    }

    public void select(int id) {
        items.forEach((key, item) -> item.setSelected(key == id));
    }

    private JComponent accountChip() {
        RoundPanel chip = new RoundPanel(new BorderLayout(12, 0), 20, Palette::field);
        chip.hover(Palette::fieldHover);
        chip.pad(8, 10, 8, 10);
        chip.setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        chip.setAlignmentX(LEFT_ALIGNMENT);
        chip.setMaximumSize(new Dimension(Integer.MAX_VALUE, 58));
        chip.setFocusable(true);
        chip.getAccessibleContext().setAccessibleName(GetText.tr("Switch account"));

        JPanel avatarWrap = new JPanel(new java.awt.GridBagLayout());
        avatarWrap.setOpaque(false);
        avatarWrap.add(chipAvatar);
        chip.add(avatarWrap, BorderLayout.WEST);

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        chipName.setAlignmentX(LEFT_ALIGNMENT);
        chipKind.setAlignmentX(LEFT_ALIGNMENT);
        text.add(Box.createVerticalGlue());
        text.add(chipName);
        text.add(chipKind);
        text.add(Box.createVerticalGlue());
        chip.add(text, BorderLayout.CENTER);

        MouseAdapter click = new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                accountMenu().show(chip, 0, -chip.getHeight() - 4 * 34);
            }
        };
        chip.addMouseListener(click);
        avatarWrap.addMouseListener(click);
        text.addMouseListener(click);
        chipName.addMouseListener(click);
        chipKind.addMouseListener(click);
        return chip;
    }

    private JPopupMenu accountMenu() {
        JPopupMenu menu = new JPopupMenu();
        MicrosoftAccount active = AccountManager.getSelectedAccount();
        for (MicrosoftAccount account : AccountManager.getAccounts()) {
            JMenuItem item = new JMenuItem(account.minecraftUsername
                + (account.offline ? "  ·  " + GetText.tr("Offline") : ""));
            if (account == active) {
                item.setIcon(Icons.CHECK.asIcon(16, Palette.accent()));
            } else {
                item.setIcon(Icons.PERSON.asIcon(16, Palette.textDim()));
            }
            item.addActionListener(e -> AccountManager.switchAccount(account));
            menu.add(item);
        }
        menu.addSeparator();
        JMenuItem manage = new JMenuItem(GetText.tr("Manage accounts"));
        manage.setIcon(Icons.SETTINGS.asIcon(16, Palette.textDim()));
        manage.addActionListener(e -> navigate.accept(UIConstants.LAUNCHER_ACCOUNTS_TAB));
        menu.add(manage);
        return menu;
    }

    private void refreshAccount() {
        MicrosoftAccount account = AccountManager.getSelectedAccount();
        chipAvatar.setAccount(account);
        chipName.setText(account == null ? GetText.tr("No account") : account.minecraftUsername);
        chipKind.setText(account == null ? GetText.tr("Add one to play")
            : account.offline ? GetText.tr("Offline player") : GetText.tr("Microsoft account"));
    }

    @Override
    public void addNotify() {
        super.addNotify();
        subscriptions.add(AccountManager.getSelectedAccountObservable()
            .subscribe(a -> SwingUtilities.invokeLater(this::refreshAccount)));
        subscriptions.add(AccountManager.getAccountsObservable()
            .subscribe(a -> SwingUtilities.invokeLater(this::refreshAccount)));
    }

    @Override
    public void removeNotify() {
        subscriptions.clear();
        super.removeNotify();
    }

    /** One row of the rail. */
    private static final class NavItem extends JComponent {
        private static final long serialVersionUID = 1L;

        private final Icons icon;
        private final String label;
        private boolean selected;
        private boolean hover;

        NavItem(Icons icon, String label, Runnable action) {
            this.icon = icon;
            this.label = label;
            setAlignmentX(LEFT_ALIGNMENT);
            setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
            setFocusable(true);
            setPreferredSize(new Dimension(WIDTH, 46));
            setMaximumSize(new Dimension(Integer.MAX_VALUE, 46));

            addMouseListener(new MouseAdapter() {
                @Override
                public void mouseClicked(MouseEvent e) {
                    action.run();
                }

                @Override
                public void mouseEntered(MouseEvent e) {
                    hover = true;
                    repaint();
                }

                @Override
                public void mouseExited(MouseEvent e) {
                    hover = false;
                    repaint();
                }
            });
            getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_ENTER, 0), "go");
            getInputMap(WHEN_FOCUSED).put(KeyStroke.getKeyStroke(KeyEvent.VK_SPACE, 0), "go");
            getActionMap().put("go", new javax.swing.AbstractAction() {
                private static final long serialVersionUID = 1L;

                @Override
                public void actionPerformed(java.awt.event.ActionEvent e) {
                    action.run();
                }
            });
            addFocusListener(new java.awt.event.FocusAdapter() {
                @Override
                public void focusGained(java.awt.event.FocusEvent e) {
                    repaint();
                }

                @Override
                public void focusLost(java.awt.event.FocusEvent e) {
                    repaint();
                }
            });
        }

        @Override
        public javax.accessibility.AccessibleContext getAccessibleContext() {
            if (accessibleContext == null) {
                accessibleContext = new AccessibleJComponent() {
                    @Override
                    public javax.accessibility.AccessibleRole getAccessibleRole() {
                        return javax.accessibility.AccessibleRole.PUSH_BUTTON;
                    }

                    @Override
                    public String getAccessibleName() {
                        return label;
                    }
                };
            }
            return accessibleContext;
        }

        void setSelected(boolean selected) {
            this.selected = selected;
            repaint();
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
            int w = getWidth();
            int h = getHeight();

            if (selected) {
                g.setColor(Palette.accentSoft());
                g.fill(new RoundRectangle2D.Float(0, 0, w, h, h, h));
            } else if (hover) {
                g.setColor(Palette.field());
                g.fill(new RoundRectangle2D.Float(0, 0, w, h, h, h));
            }
            if (isFocusOwner() && FocusRing.visible()) {
                g.setColor(Palette.accent());
                g.setStroke(new java.awt.BasicStroke(2f));
                g.draw(new RoundRectangle2D.Float(1, 1, w - 2, h - 2, h - 2, h - 2));
            }

            Color fg = selected ? Palette.accent() : hover ? Palette.text() : Palette.textDim();
            icon.paint(g, 16, (h - 22) / 2f, 22, fg);
            g.setFont(selected ? Palette.bold(14f) : Palette.font(14f));
            g.setColor(selected ? Palette.accent() : Palette.text());
            java.awt.FontMetrics fm = g.getFontMetrics();
            g.drawString(label, 52, (h - fm.getHeight()) / 2 + fm.getAscent());
            g.dispose();
        }
    }
}
