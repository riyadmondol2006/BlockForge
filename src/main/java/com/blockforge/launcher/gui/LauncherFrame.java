/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
package com.blockforge.launcher.gui;

import java.awt.BorderLayout;
import java.awt.CardLayout;
import java.awt.Component;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GraphicsConfiguration;
import java.awt.GraphicsDevice;
import java.awt.GraphicsEnvironment;
import java.awt.Insets;
import java.awt.Point;
import java.awt.Rectangle;
import java.awt.SystemTray;
import java.awt.Toolkit;
import java.awt.event.ComponentAdapter;
import java.awt.event.ComponentEvent;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;

import javax.swing.BorderFactory;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;
import javax.swing.WindowConstants;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.Pack;
import com.blockforge.launcher.evnt.listener.RelocalizationListener;
import com.blockforge.launcher.evnt.listener.ThemeListener;
import com.blockforge.launcher.evnt.manager.RelocalizationManager;
import com.blockforge.launcher.evnt.manager.TabChangeManager;
import com.blockforge.launcher.evnt.manager.ThemeManager;
import com.blockforge.launcher.gui.dialogs.InstanceInstallerDialog;
import com.blockforge.launcher.gui.modern.AccountsPage;
import com.blockforge.launcher.gui.modern.Btn;
import com.blockforge.launcher.gui.modern.DiscoverPage;
import com.blockforge.launcher.gui.modern.HomePage;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.CreatePage;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.LibraryPage;
import com.blockforge.launcher.gui.modern.Page;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.gui.modern.RoundPanel;
import com.blockforge.launcher.gui.modern.SettingsPage;
import com.blockforge.launcher.gui.modern.WhoIsPlayingDialog;
import com.blockforge.launcher.gui.modern.ToolsPage;
import com.blockforge.launcher.gui.modern.Sidebar;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.managers.PackManager;
import com.blockforge.launcher.utils.Utils;

/**
 * The main window: a navigation rail on the left and one screen at a time on the right. Screens are only created
 * the first time they're opened, which keeps start up quick.
 */
public final class LauncherFrame extends JFrame implements RelocalizationListener, ThemeListener {
    private static final long serialVersionUID = 1L;

    private final CardLayout cards = new CardLayout();
    private final JPanel stack = new JPanel(cards);
    private final Map<Integer, Supplier<Page>> factories = new HashMap<>();
    private final Map<Integer, Page> pages = new HashMap<>();
    private final JPanel runningBanner = new JPanel(new BorderLayout());
    private Sidebar sidebar;
    private int current = -1;
    /** whether the "Minecraft is running" bar should show, kept so a rebuild of the window keeps it */
    private boolean minecraftRunning;

    public LauncherFrame(boolean show) {
        LogManager.info("Launcher opening");

        App.launcher.setParentFrame(this);
        setTitle(Constants.LAUNCHER_NAME);
        setDefaultCloseOperation(WindowConstants.EXIT_ON_CLOSE);
        setResizable(true);
        setLayout(new BorderLayout());
        setIconImage(Utils.getImage("/assets/image/icon.png"));

        // a 1920x1080 laptop screen at 150% scaling only has 1280x720 to offer, less the task bar, so the window is
        // never made bigger than the screen it opens on
        Rectangle usable = usableScreen();
        setMinimumSize(fit(new Dimension(1080, 680), usable));
        setSize(fit(new Dimension(1240, 780), usable));
        setLocationRelativeTo(null);

        try {
            if (App.settings.rememberWindowSizePosition && App.settings.launcherSize != null) {
                setSize(fit(App.settings.launcherSize, usable));
            }

            // a remembered position on a screen that has since been unplugged would put the window out of reach
            if (App.settings.rememberWindowSizePosition && App.settings.launcherPosition != null
                && isReachable(App.settings.launcherPosition, getSize(), allScreens())) {
                setLocation(App.settings.launcherPosition);
            }
        } catch (Exception e) {
            LogManager.logStackTrace("Error setting custom remembered window size settings", e);
        }

        LogManager.info("Setting up interface");
        buildInterface();
        LogManager.info("Finished setting up interface");

        int start = App.settings.selectedTabOnStartup;
        navigate(factories.containsKey(start) ? start : UIConstants.LAUNCHER_NEWS_TAB);

        if (show) {
            LogManager.info("Showing Launcher");
            setVisible(true);

            // ask who is playing once the window is up
            SwingUtilities.invokeLater(() -> WhoIsPlayingDialog.askIfNeeded(this));

            addWindowListener(new WindowAdapter() {
                @Override
                public void windowClosing(WindowEvent windowEvent) {
                    try {
                        if (SystemTray.isSupported()) {
                            SystemTray.getSystemTray().remove(App.trayIcon);
                        }
                    } catch (Exception ignored) {
                        // ignored
                    }
                }
            });
        }

        RelocalizationManager.addListener(this);
        ThemeManager.addListener(this);

        if (App.packToInstall != null) {
            Pack pack = PackManager.getPackBySafeName(App.packToInstall);

            if (pack != null && pack.isSemiPublic() && !PackManager.canViewSemiPublicPackByCode(pack.getCode())) {
                LogManager.error("Error automatically installing " + pack.getName() + " as you don't have the "
                    + "pack added to the launcher!");
            } else if (pack == null) {
                LogManager.error("Error automatically installing pack!");
            } else {
                InstanceInstallerDialog instanceInstallerDialog = new InstanceInstallerDialog(pack);
                instanceInstallerDialog.setVisible(true);
            }
        }

        addComponentListener(new ComponentAdapter() {
            @Override
            public void componentResized(ComponentEvent evt) {
                Component c = (Component) evt.getSource();

                if (App.settings.rememberWindowSizePosition) {
                    App.settings.launcherSize = c.getSize();
                    App.settings.save();
                }
            }

            @Override
            public void componentMoved(ComponentEvent evt) {
                Component c = (Component) evt.getSource();

                if (App.settings.rememberWindowSizePosition) {
                    App.settings.launcherPosition = c.getLocation();
                    App.settings.save();
                }
            }
        });
    }

    /** The part of the main screen a window can use, without task bars and docks. */
    private static Rectangle usableScreen() {
        try {
            GraphicsConfiguration gc = GraphicsEnvironment.getLocalGraphicsEnvironment().getDefaultScreenDevice()
                .getDefaultConfiguration();
            Rectangle bounds = gc.getBounds();
            Insets insets = Toolkit.getDefaultToolkit().getScreenInsets(gc);
            return new Rectangle(bounds.x + insets.left, bounds.y + insets.top,
                bounds.width - insets.left - insets.right, bounds.height - insets.top - insets.bottom);
        } catch (Exception e) {
            return new Rectangle(0, 0, Integer.MAX_VALUE, Integer.MAX_VALUE);
        }
    }

    private static List<Rectangle> allScreens() {
        List<Rectangle> screens = new ArrayList<>();
        try {
            for (GraphicsDevice device : GraphicsEnvironment.getLocalGraphicsEnvironment().getScreenDevices()) {
                screens.add(device.getDefaultConfiguration().getBounds());
            }
        } catch (Exception ignored) {
            // no screens to check against
        }
        return screens;
    }

    /** The wanted size, made smaller where it doesn't fit the usable area. */
    static Dimension fit(Dimension wanted, Rectangle usable) {
        return new Dimension(Math.max(1, Math.min(wanted.width, usable.width)),
            Math.max(1, Math.min(wanted.height, usable.height)));
    }

    /** Whether enough of the window's title bar is on some screen to grab it and move it. */
    static boolean isReachable(Point location, Dimension size, List<Rectangle> screens) {
        Rectangle titleBar = new Rectangle(location.x, location.y, size.width, 32);
        for (Rectangle screen : screens) {
            Rectangle overlap = screen.intersection(titleBar);
            if (overlap.width >= 120 && overlap.height >= 16) {
                return true;
            }
        }
        return false;
    }

    private void buildInterface() {
        getContentPane().removeAll();
        stack.removeAll();
        // the old pages are thrown away, so they must let go of the view models and listeners they registered
        pages.values().forEach(Page::dispose);
        pages.clear();
        factories.clear();

        factories.put(UIConstants.LAUNCHER_NEWS_TAB, HomePage::new);
        factories.put(UIConstants.LAUNCHER_INSTANCES_TAB, LibraryPage::new);
        factories.put(UIConstants.LAUNCHER_PACKS_TAB, DiscoverPage::new);
        factories.put(UIConstants.LAUNCHER_ACCOUNTS_TAB, AccountsPage::new);
        factories.put(UIConstants.LAUNCHER_SETTINGS_TAB, SettingsPage::new);
        factories.put(UIConstants.LAUNCHER_CREATE_PACK_TAB, CreatePage::new);
        factories.put(UIConstants.LAUNCHER_TOOLS_TAB, ToolsPage::new);

        sidebar = new Sidebar(this::navigate);

        JPanel right = new JPanel(new BorderLayout());
        right.setOpaque(false);
        buildRunningBanner();
        right.add(runningBanner, BorderLayout.NORTH);
        stack.setOpaque(false);
        right.add(stack, BorderLayout.CENTER);

        JPanel root = new JPanel(new BorderLayout()) {
            private static final long serialVersionUID = 1L;

            @Override
            public java.awt.Color getBackground() {
                return Palette.bg();
            }
        };
        root.add(sidebar, BorderLayout.WEST);
        root.add(right, BorderLayout.CENTER);
        add(root, BorderLayout.CENTER);
    }

    private void buildRunningBanner() {
        runningBanner.removeAll();
        runningBanner.setOpaque(false);
        runningBanner.setBorder(BorderFactory.createEmptyBorder(14, 36, 0, 36));
        // rebuilt for a new language while the game runs: the bar has to stay up
        runningBanner.setVisible(minecraftRunning);

        RoundPanel card = new RoundPanel(new BorderLayout(14, 0), 20, Palette::accentSoft);
        card.line(Palette::accent);
        card.pad(10, 18, 10, 12);
        JLabel text = Lbl.text(GetText.tr("Minecraft is running"), 14f, true, Palette::text);
        text.setIcon(Icons.PLAY.asIcon(18, Palette::accent));
        text.setIconTextGap(10);
        card.add(text, BorderLayout.CENTER);

        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
        buttons.setOpaque(false);
        Btn console = new Btn(GetText.tr("Console"), Btn.Kind.TONAL).icon(Icons.TERMINAL).compact();
        console.addActionListener(e -> App.console.setVisible(true));
        Btn kill = new Btn(GetText.tr("Stop"), Btn.Kind.DANGER).icon(Icons.CLOSE).compact();
        kill.addActionListener(e -> {
            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Kill Minecraft") + "?")
                .setContent(GetText.tr("Are you sure you want to kill the Minecraft process? Doing so can corrupt your saves."))
                .setType(DialogManager.QUESTION).show();
            if (ret == DialogManager.YES_OPTION) {
                App.launcher.killMinecraft();
                setMinecraftRunning(false);
            }
        });
        buttons.add(console);
        buttons.add(kill);
        card.add(buttons, BorderLayout.EAST);
        runningBanner.add(card, BorderLayout.CENTER);
    }

    /** Shows or hides the "Minecraft is running" bar. */
    public void setMinecraftRunning(boolean running) {
        SwingUtilities.invokeLater(() -> {
            minecraftRunning = running;
            runningBanner.setVisible(running);
            runningBanner.revalidate();
            runningBanner.repaint();
        });
    }

    /**
     * Brings the screen for the given {@link UIConstants} destination to the front.
     */
    public void navigate(int destination) {
        Supplier<Page> factory = factories.get(destination);
        if (factory == null) {
            return;
        }

        Page page = pages.get(destination);
        if (page == null) {
            page = factory.get();
            pages.put(destination, page);
            stack.add(page, String.valueOf(destination));
        }

        cards.show(stack, String.valueOf(destination));
        sidebar.select(destination);
        page.onShow();
        if (current != destination) {
            current = destination;
            TabChangeManager.post(destination);
        }
    }

    /** Called when the available packs changed, so an open Discover screen can search again. */
    public void refreshPacks() {
        Page discover = pages.get(UIConstants.LAUNCHER_PACKS_TAB);
        if (discover instanceof DiscoverPage) {
            SwingUtilities.invokeLater(((DiscoverPage) discover)::reload);
        }
    }

    @Override
    public void onRelocalization() {
        int showing = current;
        buildInterface();
        current = -1;
        navigate(showing < 0 ? UIConstants.LAUNCHER_NEWS_TAB : showing);
        revalidate();
        repaint();
    }

    @Override
    public void onThemeChange() {
        SwingUtilities.invokeLater(() -> {
            revalidate();
            repaint();
        });
    }
}
