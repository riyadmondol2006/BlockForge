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
    /** only offered when the servers couldn't be reached; trying an empty list again changes nothing */
    private final Btn retry = new Btn(GetText.tr("Try again"), Btn.Kind.TONAL).icon(Icons.REFRESH);
    /** the query whose platform-only results (for platforms the combined search leaves out) were fetched */
    private String platformFetchedFor;
    private int platformFetched = -1;
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

        JPanel actions = FitWidthPanel.rightButtons();
        Btn custom = new Btn(GetText.tr("Custom instance"), Btn.Kind.TONAL).icon(Icons.CUBE);
        custom.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB));
        actions.add(custom);
        titleRow.add(actions, BorderLayout.EAST);
        header.add(titleRow);
        header.add(Box.createVerticalStrut(18));

        search.onChange(450, () -> load());
        search.addActionListener(e -> load());

        String[] labels = new String[PLATFORMS.length];
        for (int i = 0; i < labels.length; i++) {
            labels[i] = i == 0 ? GetText.tr("All") : PLATFORMS[i];
        }
        platformChips = new ChipRow(labels, 0, i -> render());
        FilterRow filterRow = new FilterRow(search, platformChips);
        filterRow.setAlignmentX(LEFT_ALIGNMENT);
        titleRow.setAlignmentX(LEFT_ALIGNMENT);
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
        javax.swing.JScrollPane resultsScroll = FitWidthPanel.scroll(scrollContent);
        FitWidthPanel.alignHeader(header, resultsScroll, 28, 36, 12);
        body.add(resultsScroll, "results");


        JPanel empty = new JPanel(new GridBagLayout());
        empty.setOpaque(false);
        JPanel emptyBox = new JPanel();
        emptyBox.setOpaque(false);
        emptyBox.setLayout(new BoxLayout(emptyBox, BoxLayout.Y_AXIS));
        JLabel emptyIcon = new JLabel(Icons.SEARCH.asIcon(44, Palette::textDim));
        emptyIcon.setAlignmentX(CENTER_ALIGNMENT);
        emptyText.setAlignmentX(CENTER_ALIGNMENT);
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
        final int chip = platformChips.getSelected();
        cards.show(body, "loading");

        App.TASKPOOL.execute(() -> {
            List<UnifiedModPackResultsFragment> found = null;
            try {
                found = fetch(query);
                if (found != null && chip == TECHNIC) {
                    found.addAll(technic(query, found));
                }
            } catch (Exception e) {
                com.blockforge.launcher.managers.LogManager.debug("Couldn't search for modpacks: " + e.getMessage());
            }

            List<UnifiedModPackResultsFragment> finalFound = found;
            SwingUtilities.invokeLater(() -> {
                if (ticket != request) {
                    return;
                }
                platformFetched = chip;
                platformFetchedFor = query;
                if (finalFound == null) {
                    results = new ArrayList<>();
                    emptyText.setText(GetText.tr("Couldn't reach the modpack servers. Check your connection."));
                    retry.setVisible(true);
                    cards.show(body, "empty");
                    return;
                }
                results = finalFound;
                render();
            });
        });
    }

    private static final int TECHNIC = 5;

    /**
     * Technic's own trending list or search. The combined search rarely returns Technic packs, so picking the Technic
     * chip asks Technic directly; packs already in the list aren't added twice.
     */
    private static List<UnifiedModPackResultsFragment> technic(String query,
        List<UnifiedModPackResultsFragment> already) {
        List<UnifiedModPackResultsFragment> items = new ArrayList<>();
        com.blockforge.launcher.data.technic.TechnicSearchResults found;
        try {
            found = query.isEmpty() ? com.blockforge.launcher.utils.TechnicApi.getTrendingModpacks()
                : com.blockforge.launcher.utils.TechnicApi
                    .searchModpacks(java.net.URLEncoder.encode(query, "UTF-8"));
        } catch (Exception e) {
            com.blockforge.launcher.managers.LogManager.debug("Couldn't search Technic: " + e.getMessage());
            return items;
        }
        if (found == null || found.modpacks == null) {
            return items;
        }
        java.util.Set<String> seen = new java.util.HashSet<>();
        for (UnifiedModPackResultsFragment r : already) {
            if (r.platform() == ModPackPlatformType.TECHNIC) {
                seen.add(r.id());
            }
        }
        for (com.blockforge.launcher.data.technic.TechnicModpackSlim pack : found.modpacks) {
            if (pack == null || pack.slug == null || pack.name == null || !seen.add(pack.slug)) {
                continue;
            }
            items.add(new UnifiedModPackResultsFragment("LauncherModPackResult", pack.slug, pack.name,
                ModPackPlatformType.TECHNIC, "", "https://www.technicpack.net/modpack/" + pack.slug,
                pack.iconUrl));
        }
        return items;
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

        String query = search.getText().trim();
        if (shown == 0 && chip == TECHNIC && !(platformFetched == chip && query.equals(platformFetchedFor))) {
            // nothing from Technic in the combined results: ask Technic itself
            load();
            return;
        }

        if (shown == 0) {
            retry.setVisible(false);
            if (chip != 0) {
                emptyText.setText(query.isEmpty()
                    ? GetText.tr("No {0} modpacks in this list. Search above to find more.", PLATFORMS[chip])
                    : GetText.tr("No {0} modpacks found for that search", PLATFORMS[chip]));
            } else {
                emptyText.setText(query.isEmpty() ? GetText.tr("No modpacks to show right now")
                    : GetText.tr("No modpacks found for that search"));
            }
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

    /**
     * The search box with the platform chips on the right. The box gives up width first; when even a short box and
     * the chips don't fit side by side (a narrow window), the chips move to their own line under the box.
     */
    static final class FilterRow extends JPanel {
        private static final long serialVersionUID = 1L;
        private static final int GAP = 16;
        private static final int SEARCH_MAX = 420;
        private static final int SEARCH_MIN = 220;

        private final JComponent search;
        private final JComponent chips;
        private boolean stacked;

        FilterRow(JComponent search, JComponent chips) {
            super(null);
            this.search = search;
            this.chips = chips;
            setOpaque(false);
            add(search);
            add(chips);
            addComponentListener(new java.awt.event.ComponentAdapter() {
                @Override
                public void componentResized(java.awt.event.ComponentEvent e) {
                    if (stacked != stacked(getWidth())) {
                        revalidate();
                    }
                }
            });
        }

        private boolean stacked(int width) {
            return width > 0 && width < SEARCH_MIN + GAP + chips.getPreferredSize().width;
        }

        @Override
        public Dimension getPreferredSize() {
            Dimension c = chips.getPreferredSize();
            stacked = stacked(getWidth());
            return stacked ? new Dimension(SEARCH_MIN, 44 + 10 + c.height)
                : new Dimension(SEARCH_MIN + GAP + c.width, 44);
        }

        @Override
        public Dimension getMaximumSize() {
            return new Dimension(Integer.MAX_VALUE, getPreferredSize().height);
        }

        @Override
        public void doLayout() {
            int w = getWidth();
            Dimension c = chips.getPreferredSize();
            if (stacked(w)) {
                search.setBounds(0, 0, Math.min(SEARCH_MAX, w), 44);
                chips.setBounds(0, 54, Math.min(c.width, w), c.height);
            } else {
                search.setBounds(0, 0, Math.min(SEARCH_MAX, w - GAP - c.width), 44);
                chips.setBounds(w - c.width, (44 - c.height) / 2, c.width, c.height);
            }
        }
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
                        g.clip(shape);
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
                // the whole picture is shown: logos with words in them read wrong when cropped
                ImageCache.load(result.iconUrl(), ImageCache.px(60), ImageCache.px(60), true, image -> {
                    icon = image;
                    view.repaint();
                });
                return;
            }

            // only ATLauncher results are ATLauncher packs: a CurseForge or Modrinth id can be the same number as an
            // unrelated ATLauncher pack, which would show that pack's picture
            if (!"ATLauncher".equals(platformName(result.platform()))) {
                return;
            }

            try {
                Pack pack = PackManager.getPackByID(Integer.parseInt(result.id()));
                ImageCache.async(() -> {
                    BufferedImage source = ImageCache.toBuffered(pack.getImage());
                    return source == null ? null : ImageCache.fit(source, ImageCache.px(60), ImageCache.px(60));
                }, "pack-fit:" + result.id(), image -> {
                    icon = image;
                    view.repaint();
                });
            } catch (Exception e) {
                // no icon, the placeholder stays
            }
        }
    }
}
