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
import java.awt.FlowLayout;
import java.awt.GradientPaint;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.GridBagLayout;
import java.awt.GridLayout;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.RoundRectangle2D;
import java.awt.image.BufferedImage;
import java.io.File;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JDialog;
import javax.swing.JEditorPane;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;
import javax.swing.event.HyperlinkEvent;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.AbstractNews;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.NewsManager;
import com.blockforge.launcher.utils.OS;

import io.reactivex.rxjava3.disposables.CompositeDisposable;

/**
 * The landing screen: jump back into the game, quick actions and the latest news.
 */
public class HomePage extends Page {
    private static final long serialVersionUID = 1L;

    private final JPanel column = new JPanel();
    private JPanel quickActions;
    private final JPanel heroHolder = new JPanel(new BorderLayout());
    private final JPanel recentSection = new JPanel(new BorderLayout());
    private final FitWidthPanel recentGrid = new FitWidthPanel(18);
    private final JPanel newsList = new JPanel();
    private final JLabel greeting = Lbl.title("");
    private final CompositeDisposable subscriptions = new CompositeDisposable();
    private List<Instance> instances = new ArrayList<>();

    public HomePage() {
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        column.setBorder(BorderFactory.createEmptyBorder(28, 36, 36, 36));

        greeting.setAlignmentX(LEFT_ALIGNMENT);
        JLabel sub = Lbl.dim(GetText.tr("Ready when you are."));
        sub.setAlignmentX(LEFT_ALIGNMENT);
        column.add(greeting);
        column.add(sub);
        column.add(Box.createVerticalStrut(20));

        heroHolder.setOpaque(false);
        heroHolder.setAlignmentX(LEFT_ALIGNMENT);
        column.add(heroHolder);
        column.add(Box.createVerticalStrut(18));

        quickActions = quickActions();
        column.add(quickActions);
        column.add(Box.createVerticalStrut(30));

        recentSection.setOpaque(false);
        recentSection.setAlignmentX(LEFT_ALIGNMENT);
        JPanel recentHeader = new JPanel(new BorderLayout());
        recentHeader.setOpaque(false);
        recentHeader.add(Lbl.heading(GetText.tr("Jump back in")), BorderLayout.WEST);
        Btn all = new Btn(GetText.tr("View library"), Btn.Kind.TEXT).compact();
        all.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_INSTANCES_TAB));
        recentHeader.add(all, BorderLayout.EAST);
        recentHeader.setBorder(BorderFactory.createEmptyBorder(0, 0, 10, 0));
        recentGrid.setOpaque(false);
        recentSection.add(recentHeader, BorderLayout.NORTH);
        recentSection.add(recentGrid, BorderLayout.CENTER);
        column.add(recentSection);
        column.add(Box.createVerticalStrut(30));

        JLabel newsHeading = Lbl.heading(GetText.tr("News"));
        newsHeading.setAlignmentX(LEFT_ALIGNMENT);
        column.add(newsHeading);
        column.add(Box.createVerticalStrut(10));
        newsList.setOpaque(false);
        newsList.setLayout(new BoxLayout(newsList, BoxLayout.Y_AXIS));
        newsList.setAlignmentX(LEFT_ALIGNMENT);
        column.add(newsList);

        JPanel top = new JPanel(new BorderLayout());
        top.setOpaque(false);
        top.add(column, BorderLayout.NORTH);
        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(top, BorderLayout.CENTER);
        add(FitWidthPanel.scroll(fit), BorderLayout.CENTER);
    }

    private JPanel quickActions() {
        JPanel row = new JPanel(new GridLayout(1, 3, 16, 0));
        row.setOpaque(false);
        row.setAlignmentX(LEFT_ALIGNMENT);
        row.setMaximumSize(new Dimension(Integer.MAX_VALUE, 84));
        row.add(action(Icons.EXPLORE, GetText.tr("Browse modpacks"), GetText.tr("Find something new to play"),
            () -> App.navigate(UIConstants.LAUNCHER_PACKS_TAB)));
        row.add(action(Icons.PLUS, GetText.tr("New instance"), GetText.tr("Newest Minecraft, ready to play"),
            () -> App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB)));
        row.add(action(Icons.PERSON, GetText.tr("Change skin"), GetText.tr("Pick how you look in game"),
            () -> SkinDialog.open(null)));
        return row;
    }

    private RoundPanel action(Icons icon, String title, String subtitle, Runnable run) {
        RoundPanel tile = new RoundPanel(new BorderLayout(14, 0), 22, Palette::card);
        tile.line(Palette::outline);
        tile.hover(Palette::cardHover);
        tile.pad(14, 16, 14, 16);
        tile.setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));

        JPanel badge = new JPanel() {
            private static final long serialVersionUID = 1L;

            @Override
            protected void paintComponent(Graphics g0) {
                Graphics2D g = (Graphics2D) g0.create();
                g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
                g.setColor(Palette.accentSoft());
                g.fillOval(0, 0, getWidth(), getHeight());
                icon.paint(g, (getWidth() - 22) / 2f, (getHeight() - 22) / 2f, 22, Palette.accent());
                g.dispose();
            }
        };
        badge.setOpaque(false);
        badge.setPreferredSize(new Dimension(46, 46));
        JPanel badgeWrap = new JPanel(new GridBagLayout());
        badgeWrap.setOpaque(false);
        badgeWrap.add(badge);
        tile.add(badgeWrap, BorderLayout.WEST);

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel t = Lbl.text(title, 14f, true, Palette::text);
        JLabel s = Lbl.text(subtitle, 12f, false, Palette::textDim);
        t.setAlignmentX(LEFT_ALIGNMENT);
        s.setAlignmentX(LEFT_ALIGNMENT);
        text.add(Box.createVerticalGlue());
        text.add(t);
        text.add(s);
        text.add(Box.createVerticalGlue());
        tile.add(text, BorderLayout.CENTER);

        MouseAdapter click = new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                run.run();
            }
        };
        tile.addMouseListener(click);
        badge.addMouseListener(click);
        text.addMouseListener(click);
        t.addMouseListener(click);
        s.addMouseListener(click);
        return tile;
    }

    @Override
    public void addNotify() {
        super.addNotify();
        subscriptions.add(InstanceManager.getInstancesObservable().subscribe(list -> SwingUtilities
            .invokeLater(() -> {
                instances = new ArrayList<>(list);
                refreshInstances();
            })));
        subscriptions.add(NewsManager.getNews().subscribe(news -> SwingUtilities.invokeLater(() -> showNews(news))));
        // the greeting names the player, so it follows account switches made from the sidebar
        subscriptions.add(AccountManager.getSelectedAccountObservable()
            .subscribe(account -> SwingUtilities.invokeLater(() -> MicrosoftAccountName.apply(greeting))));
    }

    @Override
    public void removeNotify() {
        subscriptions.clear();
        super.removeNotify();
    }

    @Override
    public void onShow() {
        refreshInstances();
    }

    private void refreshInstances() {
        MicrosoftAccountName.apply(greeting);

        List<Instance> byRecent = new ArrayList<>(instances);
        byRecent.sort(Comparator.comparing(Instance::getLastPlayedOrEpoch).reversed());

        heroHolder.removeAll();
        heroHolder.add(byRecent.isEmpty() ? new Hero(null) : new Hero(byRecent.get(0)), BorderLayout.CENTER);

        recentGrid.removeAll();
        int count = 0;
        for (int i = 1; i < byRecent.size() && count < 4; i++, count++) {
            Instance instance = byRecent.get(i);
            InstanceTile[] self = new InstanceTile[1];
            self[0] = new InstanceTile(instance, () -> {
                self[0].refresh();
                refreshInstances();
            });
            recentGrid.add(self[0]);
        }
        recentSection.setVisible(count > 0);

        heroHolder.revalidate();
        heroHolder.repaint();
        recentGrid.revalidate();
        recentGrid.repaint();
    }

    /** Fills the greeting with the name of the player that is in use. */
    private static final class MicrosoftAccountName {
        static void apply(JLabel label) {
            com.blockforge.launcher.data.MicrosoftAccount account = AccountManager.getSelectedAccount();
            label.setText(account == null ? GetText.tr("Welcome to BlockForge")
                : GetText.tr("Welcome back, {0}", account.minecraftUsername));
        }
    }

    private static final Pattern TITLE = Pattern.compile("<h2>(.*?)</h2>", Pattern.DOTALL);

    private void showNews(List<AbstractNews> news) {
        newsList.removeAll();
        if (news == null || news.isEmpty()) {
            JLabel none = Lbl.dim(GetText.tr("No news right now."));
            none.setAlignmentX(LEFT_ALIGNMENT);
            newsList.add(none);
        } else {
            int shown = 0;
            for (AbstractNews item : news) {
                if (shown++ >= 6) {
                    break;
                }
                newsList.add(newsCard(item));
                newsList.add(Box.createVerticalStrut(10));
            }
        }
        newsList.revalidate();
        newsList.repaint();
    }

    private RoundPanel newsCard(AbstractNews news) {
        String html = news.htmlEntry == null ? "" : news.htmlEntry;
        Matcher m = TITLE.matcher(html);
        String title = m.find() ? DiscoverPage.plain(m.group(1)) : GetText.tr("News");
        String body = DiscoverPage.plain(m.replaceFirst(""));

        RoundPanel card = new RoundPanel(new BorderLayout(0, 6), 20, Palette::card);
        card.line(Palette::outline);
        card.hover(Palette::cardHover);
        card.pad(14, 18, 14, 18);
        card.setAlignmentX(LEFT_ALIGNMENT);
        card.setMaximumSize(new Dimension(Integer.MAX_VALUE, 130));
        card.setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
        card.add(Lbl.text(title, 15f, true, Palette::text), BorderLayout.NORTH);
        card.add(new Lbl.Paragraph(body, 13f, 2, 900, Palette::textDim), BorderLayout.CENTER);
        card.addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                openNews(title, html);
            }
        });
        return card;
    }

    private void openNews(String title, String html) {
        JDialog dialog = new JDialog(App.launcher.getParent(), title, java.awt.Dialog.ModalityType.DOCUMENT_MODAL);
        JEditorPane pane = new JEditorPane("text/html", html);
        pane.setEditable(false);
        pane.addHyperlinkListener(e -> {
            if (e.getEventType() == HyperlinkEvent.EventType.ACTIVATED) {
                OS.openWebBrowser(e.getURL());
            }
        });
        pane.setCaretPosition(0);
        dialog.add(FitWidthPanel.scroll(pane));
        dialog.setSize(760, 520);
        dialog.setLocationRelativeTo(App.launcher.getParent());
        dialog.setVisible(true);
    }

    /** The big card at the top: continue with the last instance, or get started. */
    private static final class Hero extends JPanel {
        private static final long serialVersionUID = 1L;

        private BufferedImage image;

        Hero(Instance instance) {
            super(new BorderLayout());
            setOpaque(false);
            if (instance != null) {
                // files dropped on the "continue playing" banner go into that instance
                putClientProperty(FileDropFlow.INSTANCE_KEY, instance);
            }
            setPreferredSize(new Dimension(0, 200));
            setMinimumSize(new Dimension(0, 200));
            setMaximumSize(new Dimension(Integer.MAX_VALUE, 200));

            JPanel text = new JPanel();
            text.setOpaque(false);
            text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
            text.setBorder(BorderFactory.createEmptyBorder(26, 32, 26, 32));

            JLabel kicker = Lbl.text(instance == null ? GetText.tr("GET STARTED") : GetText.tr("CONTINUE PLAYING"),
                11f, true, Palette::accent);
            JLabel title = Lbl.text(instance == null ? GetText.tr("Start your first adventure")
                : instance.launcher.name, 28f, true, Palette::text);
            JLabel sub = Lbl.text(instance == null
                ? GetText.tr("Install a modpack or build your own instance. No account needed.")
                : details(instance), 14f, false, Palette::textDim);
            kicker.setAlignmentX(LEFT_ALIGNMENT);
            title.setAlignmentX(LEFT_ALIGNMENT);
            sub.setAlignmentX(LEFT_ALIGNMENT);
            text.add(kicker);
            text.add(Box.createVerticalStrut(6));
            text.add(title);
            text.add(Box.createVerticalStrut(4));
            text.add(sub);
            text.add(Box.createVerticalGlue());

            JPanel buttons = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
            buttons.setOpaque(false);
            buttons.setAlignmentX(LEFT_ALIGNMENT);
            if (instance == null) {
                Btn browse = new Btn(GetText.tr("Browse modpacks"), Btn.Kind.FILLED).icon(Icons.EXPLORE);
                browse.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_PACKS_TAB));
                Btn create = new Btn(GetText.tr("New instance"), Btn.Kind.TONAL).icon(Icons.PLUS);
                create.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB));
                buttons.add(browse);
                buttons.add(Box.createHorizontalStrut(10));
                buttons.add(create);
            } else {
                Btn play = new Btn(GetText.tr("Play"), Btn.Kind.FILLED).icon(Icons.PLAY);
                play.addActionListener(e -> InstanceActions.play(instance, false, false));
                Btn settings = new Btn(GetText.tr("Settings"), Btn.Kind.TONAL).icon(Icons.SETTINGS);
                settings.addActionListener(e -> InstanceActions.openSettings(instance));
                Btn mods = new Btn(GetText.tr("Mods and packs"), Btn.Kind.TONAL).icon(Icons.PUZZLE);
                mods.addActionListener(e -> InstanceActions.openContent(instance,
                    com.blockforge.launcher.content.ContentKind.MODS, null));
                buttons.add(play);
                buttons.add(Box.createHorizontalStrut(10));
                buttons.add(mods);
                buttons.add(Box.createHorizontalStrut(10));
                buttons.add(settings);
            }
            text.add(buttons);
            add(text, BorderLayout.WEST);

            if (instance != null) {
                File custom = instance.getRoot().resolve("instance.png").toFile();
                ImageCache.async(() -> {
                    BufferedImage source = ImageCache.toBuffered(instance.getImage());
                    return source == null ? null : ImageCache.cover(source, 900, 400);
                }, "hero:" + instance.getRoot() + "#" + custom.lastModified(), img -> {
                    image = img;
                    faded = null;
                    repaint();
                });
            }
        }

        private BufferedImage faded;

        /** The picture with its left edge fading into transparency, so it melts into whatever is behind it. */
        private BufferedImage fadedImage(int iw, int h) {
            if (faded == null || faded.getWidth() != iw || faded.getHeight() != h) {
                faded = new BufferedImage(iw, h, BufferedImage.TYPE_INT_ARGB);
                Graphics2D fg = faded.createGraphics();
                fg.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
                fg.drawImage(image, 0, 0, iw, h, null);
                fg.setComposite(java.awt.AlphaComposite.DstIn);
                fg.setPaint(new GradientPaint(0, 0, new Color(0, 0, 0, 0), iw * 0.55f, 0, new Color(0, 0, 0, 255)));
                fg.fillRect(0, 0, iw, h);
                fg.dispose();
            }
            return faded;
        }

        private String details(Instance instance) {
            StringBuilder sb = new StringBuilder("Minecraft ").append(instance.id);
            if (instance.launcher.loaderVersion != null && instance.launcher.loaderVersion.type != null) {
                sb.append(" · ").append(instance.launcher.loaderVersion.type);
            }
            if (instance.launcher.pack != null && !instance.launcher.pack.equals(instance.launcher.name)) {
                sb.append(" · ").append(instance.launcher.pack);
            }
            return sb.toString();
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
            int w = getWidth();
            int h = getHeight();
            // the clip handed in keeps painting inside the scroll pane, so it is narrowed and later put back,
            // never replaced (setClip(null) let a half scrolled card paint over the window around it)
            java.awt.Shape visible = g.getClip();
            g.clip(new RoundRectangle2D.Float(0, 0, w, h, 30, 30));

            Color a = Palette.mix(Palette.card(), Palette.accent(), Palette.dark() ? 0.22f : 0.12f);
            g.setPaint(new GradientPaint(0, 0, a, w, h, Palette.card()));
            g.fillRect(0, 0, w, h);

            if (image != null) {
                int iw = Math.min(w / 2, 560);
                g.drawImage(fadedImage(iw, h), w - iw, 0, null);
            } else {
                Icons.CUBE.paint(g, w - 190, (h - 110) / 2f, 110, Palette.alpha(Palette.accent(), 70));
            }

            g.setClip(visible);
            g.setColor(Palette.outline());
            g.draw(new RoundRectangle2D.Float(0.5f, 0.5f, w - 1, h - 1, 30, 30));
            g.dispose();
        }
    }
}
