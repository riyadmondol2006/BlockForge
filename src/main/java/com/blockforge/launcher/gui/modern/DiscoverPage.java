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
import java.awt.CardLayout;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.awt.image.BufferedImage;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.Pack;
import com.blockforge.launcher.graphql.UnifiedModPackHomeQuery;
import com.blockforge.launcher.graphql.UnifiedModPackSearchQuery;
import com.blockforge.launcher.graphql.fragment.UnifiedModPackResultsFragment;
import com.blockforge.launcher.graphql.type.ModPackPlatformType;
import com.blockforge.launcher.gui.dialogs.InstanceInstallerDialog;
import com.blockforge.launcher.managers.PackManager;
import com.blockforge.launcher.network.GraphqlClient;
import com.blockforge.launcher.utils.OS;

/**
 * Find modpacks across every platform in one place and install them as an instance.
 */
public class DiscoverPage extends Page {
    private static final long serialVersionUID = 1L;

    private static final String[] PLATFORMS = { "All", "ATLauncher", "CurseForge", "Modrinth", "FTB", "Technic" };

    private final SearchBox search = new SearchBox(GetText.tr("Search modpacks"), true, 24);
    private final ChipRow platformChips;
    private final CardLayout cards = new CardLayout();
    private final JPanel body = new JPanel(cards);
    private final FitWidthPanel grid = new FitWidthPanel(18);
    private final JLabel emptyText = Lbl.dim("");
    private List<UnifiedModPackResultsFragment> results = new ArrayList<>();
    private int request;
    private boolean loadedOnce;

    public DiscoverPage() {
        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        header.setBorder(BorderFactory.createEmptyBorder(28, 36, 12, 36));

        JPanel titleRow = new JPanel(new BorderLayout());
        titleRow.setOpaque(false);
        JPanel titles = new JPanel();
        titles.setOpaque(false);
        titles.setLayout(new BoxLayout(titles, BoxLayout.Y_AXIS));
        JLabel title = Lbl.title(GetText.tr("Discover"));
        JLabel sub = Lbl.dim(GetText.tr("Modpacks from ATLauncher, CurseForge, Modrinth, FTB and Technic"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        titles.add(title);
        titles.add(sub);
        titleRow.add(titles, BorderLayout.WEST);

        JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        actions.setOpaque(false);
        Btn custom = new Btn(GetText.tr("Custom instance"), Btn.Kind.TONAL).icon(Icons.CUBE);
        custom.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB));
        actions.add(custom);
        titleRow.add(actions, BorderLayout.EAST);
        header.add(titleRow);
        header.add(Box.createVerticalStrut(18));

        JPanel filterRow = new JPanel(new BorderLayout(16, 0));
        filterRow.setOpaque(false);
        search.setPreferredSize(new Dimension(420, 44));
        search.onChange(450, () -> load());
        search.addActionListener(e -> load());
        JPanel searchWrap = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        searchWrap.setOpaque(false);
        searchWrap.add(search);
        filterRow.add(searchWrap, BorderLayout.WEST);

        String[] labels = new String[PLATFORMS.length];
        for (int i = 0; i < labels.length; i++) {
            labels[i] = i == 0 ? GetText.tr("All") : PLATFORMS[i];
        }
        platformChips = new ChipRow(labels, 0, i -> render());
        JPanel chipWrap = new JPanel(new FlowLayout(FlowLayout.RIGHT, 0, 4));
        chipWrap.setOpaque(false);
        chipWrap.add(platformChips);
        filterRow.add(chipWrap, BorderLayout.EAST);
        header.add(filterRow);
        add(header, BorderLayout.NORTH);

        body.setOpaque(false);

        JPanel loading = new JPanel(new GridBagLayout());
        loading.setOpaque(false);
        loading.add(new Spinner(44));
        body.add(loading, "loading");

        JPanel gridWrap = new JPanel(new BorderLayout());
        gridWrap.setOpaque(false);
        gridWrap.setBorder(BorderFactory.createEmptyBorder(8, 36, 36, 36));
        gridWrap.add(grid, BorderLayout.NORTH);
        FitWidthPanel scrollContent = new FitWidthPanel(0);
        scrollContent.setLayout(new BorderLayout());
        scrollContent.add(gridWrap, BorderLayout.CENTER);
        body.add(FitWidthPanel.scroll(scrollContent), "results");

        JPanel empty = new JPanel(new GridBagLayout());
        empty.setOpaque(false);
        JPanel emptyBox = new JPanel();
        emptyBox.setOpaque(false);
        emptyBox.setLayout(new BoxLayout(emptyBox, BoxLayout.Y_AXIS));
        JLabel emptyIcon = new JLabel(Icons.SEARCH.asIcon(44, Palette.textDim()));
        emptyIcon.setAlignmentX(CENTER_ALIGNMENT);
        emptyText.setAlignmentX(CENTER_ALIGNMENT);
        Btn retry = new Btn(GetText.tr("Try again"), Btn.Kind.TONAL).icon(Icons.REFRESH);
        retry.setAlignmentX(CENTER_ALIGNMENT);
        retry.addActionListener(e -> load());
        emptyBox.add(emptyIcon);
        emptyBox.add(Box.createVerticalStrut(12));
        emptyBox.add(emptyText);
        emptyBox.add(Box.createVerticalStrut(16));
        emptyBox.add(retry);
        empty.add(emptyBox, new GridBagConstraints());
        body.add(empty, "empty");

        add(body, BorderLayout.CENTER);
    }

    @Override
    public void onShow() {
        if (!loadedOnce) {
            loadedOnce = true;
            load();
        }
    }

    /** Searches again with the current query. */
    public void reload() {
        if (loadedOnce) {
            load();
        }
    }

    private void load() {
        final String query = search.getText().trim();
        if (query.length() == 1) {
            return;
        }

        final int ticket = ++request;
        cards.show(body, "loading");

        App.TASKPOOL.execute(() -> {
            List<UnifiedModPackResultsFragment> found = null;
            try {
                found = fetch(query);
            } catch (Exception e) {
                com.blockforge.launcher.managers.LogManager.debug("Couldn't search for modpacks: " + e.getMessage());
            }

            List<UnifiedModPackResultsFragment> finalFound = found;
            SwingUtilities.invokeLater(() -> {
                if (ticket != request) {
                    return;
                }
                if (finalFound == null) {
                    results = new ArrayList<>();
                    emptyText.setText(GetText.tr("Couldn't reach the modpack servers. Check your connection."));
                    cards.show(body, "empty");
                    return;
                }
                results = finalFound;
                render();
            });
        });
    }

    private static List<UnifiedModPackResultsFragment> fetch(String query) {
        List<UnifiedModPackResultsFragment> items = new ArrayList<>();
        if (query.isEmpty()) {
            UnifiedModPackHomeQuery.Data response = GraphqlClient.callAndWait(new UnifiedModPackHomeQuery());
            if (response == null || response.unifiedModPackHome() == null) {
                return null;
            }
            response.unifiedModPackHome().forEach(i -> items.add(i.fragments().unifiedModPackResultsFragment()));
        } else {
            UnifiedModPackSearchQuery.Data response = GraphqlClient
                .callAndWait(new UnifiedModPackSearchQuery(query));
            if (response == null || response.unifiedModPackSearch() == null) {
                return null;
            }
            response.unifiedModPackSearch().forEach(i -> items.add(i.fragments().unifiedModPackResultsFragment()));
        }
        return items;
    }

    private static String platformName(ModPackPlatformType type) {
        String raw = type == null ? "" : type.toString().toLowerCase(Locale.ROOT);
        if (raw.contains("curse")) {
            return "CurseForge";
        }
        if (raw.contains("modrinth")) {
            return "Modrinth";
        }
        if (raw.contains("ftb")) {
            return "FTB";
        }
        if (raw.contains("technic")) {
            return "Technic";
        }
        if (raw.contains("atl")) {
            return "ATLauncher";
        }
        return "";
    }

    private void render() {
        int chip = platformChips.getSelected();
        grid.removeAll();
        int shown = 0;
        for (UnifiedModPackResultsFragment result : results) {
            if (chip != 0 && !platformName(result.platform()).equals(PLATFORMS[chip])) {
                continue;
            }
            grid.add(new PackTile(result));
            shown++;
        }

        if (shown == 0) {
            emptyText.setText(search.getText().trim().isEmpty() ? GetText.tr("Nothing to show here yet")
                : GetText.tr("No modpacks found for that search"));
            cards.show(body, "empty");
        } else {
            cards.show(body, "results");
        }
        grid.revalidate();
        grid.repaint();
    }

    /** Strips markdown and html so that summaries read as plain text. */
    static String plain(String text) {
        if (text == null) {
            return "";
        }
        return text.replaceAll("<[^>]+>", " ").replaceAll("!?\\[([^\\]]*)\\]\\([^)]*\\)", "$1")
            .replaceAll("[*_`#>]+", "").replaceAll("&nbsp;", " ").replaceAll("[ \\t]+", " ").trim();
    }

    /** One modpack search result. */
    private static final class PackTile extends RoundPanel {
        private static final long serialVersionUID = 1L;

        private BufferedImage icon;

        PackTile(UnifiedModPackResultsFragment result) {
            super(new BorderLayout(0, 12), 22, Palette::card);
            line(Palette::outline);
            hover(Palette::cardHover);
            pad(16, 16, 16, 16);
            setPreferredSize(new Dimension(284, 236));

            JComponent iconView = new JComponent() {
                private static final long serialVersionUID = 1L;

                @Override
                public Dimension getPreferredSize() {
                    return new Dimension(60, 60);
                }

                @Override
                protected void paintComponent(java.awt.Graphics g0) {
                    java.awt.Graphics2D g = (java.awt.Graphics2D) g0.create();
                    g.setRenderingHint(java.awt.RenderingHints.KEY_ANTIALIASING,
                        java.awt.RenderingHints.VALUE_ANTIALIAS_ON);
                    java.awt.geom.RoundRectangle2D shape = new java.awt.geom.RoundRectangle2D.Float(0, 0, 60, 60,
                        16, 16);
                    g.setColor(Palette.field());
                    g.fill(shape);
                    if (icon != null) {
                        g.setClip(shape);
                        g.drawImage(icon, 0, 0, 60, 60, null);
                    } else {
                        Icons.CUBE.paint(g, 16, 16, 28, Palette.textDim());
                    }
                    g.dispose();
                }
            };

            JPanel names = new JPanel();
            names.setOpaque(false);
            names.setLayout(new BoxLayout(names, BoxLayout.Y_AXIS));
            JLabel name = Lbl.text(result.name(), 15f, true, Palette::text);
            name.setAlignmentX(LEFT_ALIGNMENT);
            JLabel platform = Lbl.text(platformName(result.platform()), 12f, true, Palette::accent);
            platform.setAlignmentX(LEFT_ALIGNMENT);
            names.add(Box.createVerticalGlue());
            names.add(name);
            names.add(platform);
            names.add(Box.createVerticalGlue());

            JPanel top = new JPanel(new BorderLayout(14, 0));
            top.setOpaque(false);
            top.add(iconView, BorderLayout.WEST);
            top.add(names, BorderLayout.CENTER);
            add(top, BorderLayout.NORTH);

            add(new Lbl.Paragraph(plain(result.summary()), 12.5f, 4, 250, Palette::textDim), BorderLayout.CENTER);

            Btn install = new Btn(GetText.tr("Install"), Btn.Kind.FILLED).icon(Icons.DOWNLOAD).compact();
            install.addActionListener(e -> new InstanceInstallerDialog(result, false).setVisible(true));
            Btn website = Btn.iconButton(Icons.EXTERNAL, GetText.tr("Open website"));
            website.addActionListener(e -> OS.openWebBrowser(result.url()));
            JPanel bottom = new JPanel(new BorderLayout());
            bottom.setOpaque(false);
            bottom.add(install, BorderLayout.WEST);
            bottom.add(website, BorderLayout.EAST);
            add(bottom, BorderLayout.SOUTH);

            loadIcon(result, iconView);
        }

        private void loadIcon(UnifiedModPackResultsFragment result, JComponent view) {
            if (result.iconUrl() != null && !result.iconUrl().isEmpty()) {
                ImageCache.load(result.iconUrl(), 120, 120, image -> {
                    icon = image;
                    view.repaint();
                });
                return;
            }

            try {
                Pack pack = PackManager.getPackByID(Integer.parseInt(result.id()));
                ImageCache.async(() -> {
                    BufferedImage source = ImageCache.toBuffered(pack.getImage());
                    return source == null ? null : ImageCache.cover(source, 120, 120);
                }, "pack:" + result.id(), image -> {
                    icon = image;
                    view.repaint();
                });
            } catch (Exception e) {
                // no icon, the placeholder stays
            }
        }
    }
}
