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
import java.awt.Component;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.content.ContentKind;
import com.blockforge.launcher.content.ContentService;
import com.blockforge.launcher.content.LoaderSupport;
import com.blockforge.launcher.content.PerformancePack;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.modrinth.ModrinthSearchHit;
import com.blockforge.launcher.data.modrinth.ModrinthSearchResult;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.utils.ComboItem;

/** One tab of the content manager: what's installed of one kind, and Modrinth to find more. */
final class ContentPanel extends JPanel {
    private static final long serialVersionUID = 1L;

    private static final int INSTALLED = 0;
    private static final int BROWSE = 1;

    final ContentKind kind;
    private final ContentManagerDialog owner;
    private final ContentService service;

    private final ChipRow mode;
    private final CardLayout searchCards = new CardLayout();
    private final JPanel searchSlot = new JPanel(searchCards);
    private final CardLayout actionCards = new CardLayout();
    private final JPanel actionSlot = new JPanel(actionCards);
    private final CardLayout centerCards = new CardLayout();
    private final JPanel center = new JPanel(centerCards);

    private final SearchBox filter;
    private final SearchBox search;
    private final JComboBox<ComboItem<String>> sort = new JComboBox<>();

    // installed
    private final JPanel banner = new JPanel(new BorderLayout());
    private final CardLayout installedCards = new CardLayout();
    private final JPanel installedBody = new JPanel(installedCards);
    private final FitWidthPanel list = new FitWidthPanel(0);
    private final JPanel empty = new JPanel(new GridBagLayout());
    private final Btn updateAll = new Btn("", Btn.Kind.TONAL).icon(Icons.DOWNLOAD);
    private final Btn performance = new Btn(GetText.tr("Performance mods"), Btn.Kind.TONAL).icon(Icons.BOLT);
    private IdentityHashMap<DisableableMod, ModrinthVersion> updates = new IdentityHashMap<>();
    private List<DisableableMod> items = new ArrayList<>();

    // browse
    private final CardLayout browseCards = new CardLayout();
    private final JPanel browseBody = new JPanel(browseCards);
    private final FitWidthPanel grid = new FitWidthPanel(16);
    private final JPanel browseMessage = new JPanel(new GridBagLayout());
    private final Btn loadMore = new Btn(GetText.tr("Show more"), Btn.Kind.TONAL);
    private int page;
    private int ticket;
    private boolean browsedOnce;

    ContentPanel(ContentManagerDialog owner, ContentKind kind) {
        super(new BorderLayout(0, 14));
        this.owner = owner;
        this.kind = kind;
        this.service = owner.service();
        setOpaque(false);

        filter = new SearchBox(filterHint(), true, 18);
        search = new SearchBox(searchHint(), true, 24);

        mode = new ChipRow(new String[] { GetText.tr("Installed"), GetText.tr("Browse Modrinth") }, INSTALLED,
            i -> showMode(i));

        add(toolbar(), BorderLayout.NORTH);

        center.setOpaque(false);
        center.add(installedView(), "installed");
        center.add(browseView(), "browse");
        add(center, BorderLayout.CENTER);
    }

    private String noun() {
        switch (kind) {
            case RESOURCE_PACKS:
                return GetText.tr("resource packs");
            case SHADERS:
                return GetText.tr("shaders");
            default:
                return GetText.tr("mods");
        }
    }

    private String filterHint() {
        return GetText.tr("Filter installed {0}", noun());
    }

    private String searchHint() {
        return GetText.tr("Search Modrinth for {0}", noun());
    }

    // ---------------------------------------------------------------- top row

    private JComponent toolbar() {
        JPanel row = new JPanel(new BorderLayout(14, 0));
        row.setOpaque(false);

        JPanel chips = new JPanel(new GridBagLayout());
        chips.setOpaque(false);
        chips.add(mode);
        row.add(chips, BorderLayout.WEST);

        filter.onChange(150, this::renderInstalled);
        search.onChange(450, () -> browse(true));
        search.addActionListener(e -> browse(true));
        searchSlot.setOpaque(false);
        searchSlot.add(fit(filter, 340), "installed");
        searchSlot.add(fit(search, 420), "browse");
        row.add(searchSlot, BorderLayout.CENTER);

        actionSlot.setOpaque(false);
        actionSlot.add(installedActions(), "installed");
        actionSlot.add(browseActions(), "browse");
        row.add(actionSlot, BorderLayout.EAST);
        return row;
    }

    /** Lets a search box grow to a width and no further. */
    private static JComponent fit(SearchBox box, int max) {
        JPanel wrap = new JPanel(new BorderLayout()) {
            private static final long serialVersionUID = 1L;

            @Override
            public void doLayout() {
                int w = Math.min(max, getWidth());
                box.setBounds(0, (getHeight() - 44) / 2, Math.max(120, w), 44);
            }

            @Override
            public Dimension getPreferredSize() {
                return new Dimension(Math.min(max, 260), 44);
            }
        };
        wrap.setOpaque(false);
        wrap.add(box);
        return wrap;
    }

    private JComponent installedActions() {
        JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 4));
        actions.setOpaque(false);

        updateAll.setVisible(false);
        updateAll.addActionListener(e -> owner.runUpdates(this, new IdentityHashMap<>(updates)));
        actions.add(updateAll);

        boolean offerPerformance = kind == ContentKind.MODS && PerformancePack.offeredFor(service.loaderKey());
        performance.setVisible(offerPerformance);
        performance.setToolTipText(GetText.tr("Adds {0}: they make Minecraft run faster and don't change the game.",
            PerformancePack.describe(service.loaderKey())));
        performance.addActionListener(e -> owner.runPerformancePack(performance));
        actions.add(performance);

        Btn files = new Btn(GetText.tr("Add files"), Btn.Kind.TONAL).icon(Icons.IMPORT);
        files.setToolTipText(GetText.tr("Add .jar or .zip files from your computer. You can also drop them here."));
        files.addActionListener(e -> owner.chooseFiles(kind));
        actions.add(files);
        return actions;
    }

    private JComponent browseActions() {
        JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 4));
        actions.setOpaque(false);
        sort.addItem(new ComboItem<>("relevance", GetText.tr("Best match")));
        sort.addItem(new ComboItem<>("downloads", GetText.tr("Most downloaded")));
        sort.addItem(new ComboItem<>("updated", GetText.tr("Recently updated")));
        sort.addItem(new ComboItem<>("newest", GetText.tr("Newest")));
        SearchBox.styleLike(sort, 190);
        sort.addActionListener(e -> {
            if (browsedOnce) {
                browse(true);
            }
        });
        actions.add(sort);

        Btn classic = new Btn(GetText.tr("More sources"), Btn.Kind.TEXT).icon(Icons.EXTERNAL);
        classic.setToolTipText(GetText.tr("CurseForge, data packs and worlds, in the classic window"));
        classic.addActionListener(e -> owner.openClassicBrowser());
        actions.add(classic);
        return actions;
    }

    void showMode(int index) {
        mode.setSelected(index);
        boolean browse = index == BROWSE;
        searchCards.show(searchSlot, browse ? "browse" : "installed");
        actionCards.show(actionSlot, browse ? "browse" : "installed");
        centerCards.show(center, browse ? "browse" : "installed");
        if (browse && !browsedOnce) {
            browse(true);
        }
        (browse ? search : filter).requestFocusInWindow();
    }

    boolean isBrowsing() {
        return mode.getSelected() == BROWSE;
    }

    // ---------------------------------------------------------------- installed

    private JComponent installedView() {
        JPanel view = new JPanel(new BorderLayout(0, 12));
        view.setOpaque(false);
        banner.setOpaque(false);
        view.add(banner, BorderLayout.NORTH);

        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        JPanel listWrap = new JPanel(new BorderLayout());
        listWrap.setOpaque(false);
        listWrap.add(list, BorderLayout.NORTH);
        FitWidthPanel scrollContent = new FitWidthPanel(0);
        scrollContent.setLayout(new BorderLayout());
        scrollContent.add(listWrap, BorderLayout.CENTER);
        JScrollPane scroll = FitWidthPanel.scroll(scrollContent);
        scroll.getVerticalScrollBar().setUnitIncrement(24);

        empty.setOpaque(false);
        JPanel loading = new JPanel(new GridBagLayout());
        loading.setOpaque(false);
        loading.add(new Spinner(40));

        installedBody.setOpaque(false);
        installedBody.add(loading, "loading");
        installedBody.add(scroll, "list");
        installedBody.add(FitWidthPanel.scroll(wrapTop(empty)), "empty");
        view.add(installedBody, BorderLayout.CENTER);
        return view;
    }

    private static JComponent wrapTop(JComponent c) {
        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(c, BorderLayout.NORTH);
        return fit;
    }

    /** Reads the instance again and redraws the list. Call on the interface thread. */
    void refreshInstalled() {
        items = service.installed(kind);
        updates.keySet().retainAll(items);
        renderBanner();
        renderInstalled();
    }

    int count() {
        return items.size();
    }

    void setUpdates(Map<DisableableMod, ModrinthVersion> found) {
        updates = new IdentityHashMap<>(found);
        renderInstalled();
    }

    private void renderInstalled() {
        String query = filter.getText().trim().toLowerCase(Locale.ROOT);
        // two items with the same name (a file kept twice) are told apart by their file names
        java.util.Set<String> seen = new java.util.HashSet<>();
        java.util.Set<String> twice = new java.util.HashSet<>();
        for (DisableableMod mod : items) {
            String key = ContentService.nameOf(mod).toLowerCase(Locale.ROOT);
            if (!seen.add(key)) {
                twice.add(key);
            }
        }
        list.removeAll();
        int shown = 0;
        for (DisableableMod mod : items) {
            String name = ContentService.nameOf(mod).toLowerCase(Locale.ROOT);
            if (!query.isEmpty() && !name.contains(query)
                && (mod.file == null || !mod.file.toLowerCase(Locale.ROOT).contains(query))) {
                continue;
            }
            if (shown > 0) {
                list.add(Box.createVerticalStrut(8));
            }
            list.add(new ContentCards.Row(owner, kind, mod, updates.get(mod), twice.contains(name)));
            shown++;
        }
        if (items.isEmpty()) {
            renderEmpty();
            installedCards.show(installedBody, "empty");
        } else if (shown == 0) {
            JLabel none = Lbl.dim(GetText.tr("Nothing installed matches \"{0}\"", filter.getText().trim()));
            none.setAlignmentX(LEFT_ALIGNMENT);
            list.add(none);
            installedCards.show(installedBody, "list");
        } else {
            installedCards.show(installedBody, "list");
        }

        int n = updates.size();
        updateAll.setText(GetText.tr("Update all ({0})", n));
        updateAll.setVisible(n > 0);
        list.revalidate();
        list.repaint();
    }

    void showLoadingInstalled() {
        installedCards.show(installedBody, "loading");
    }

    // ---------------------------------------------------------------- banners and empty states

    private void renderBanner() {
        banner.removeAll();
        JComponent content = null;
        String loader = service.loaderKey();
        if (kind == ContentKind.SHADERS) {
            switch (service.shaderSetup()) {
                case ADD_IRIS:
                    content = bannerCard(Icons.WARNING, GetText.tr("Shaders need Iris to work"),
                        GetText.tr("Iris and Sodium will be added from Modrinth. They make shaders run fast."),
                        button(GetText.tr("Add Iris"), Icons.DOWNLOAD, b -> owner.runShaderLoader(b)));
                    break;
                case ADD_OCULUS:
                    content = bannerCard(Icons.WARNING, GetText.tr("Shaders on Forge need Oculus or OptiFine"),
                        GetText.tr("Oculus can be added from Modrinth if it's made for Minecraft {0}.",
                            service.minecraftVersion()),
                        button(GetText.tr("Add Oculus"), Icons.DOWNLOAD, b -> owner.runShaderLoader(b)));
                    break;
                case NEEDS_MOD_LOADER:
                    content = bannerCard(Icons.WARNING, GetText.tr("Shaders need a mod loader"),
                        GetText.tr("Add Fabric to this instance, then Iris, and your shaders will work."),
                        button(GetText.tr("Add Fabric"), Icons.PLUS, b -> owner.addFabric()));
                    break;
                default:
                    if (!items.isEmpty()) {
                        content = bannerCard(Icons.INFO, null,
                            GetText.tr("Pick a shader in Minecraft: Options, Video Settings, Shader Packs."), null);
                    }
                    break;
            }
        } else if (kind == ContentKind.MODS && loader == null) {
            content = bannerCard(Icons.WARNING, GetText.tr("This instance has no mod loader"),
                GetText.tr("Mods need a loader such as Fabric. Add it here; your worlds stay as they are."),
                button(GetText.tr("Add Fabric"), Icons.PLUS, b -> owner.addFabric()));
        } else if (kind == ContentKind.RESOURCE_PACKS && !items.isEmpty()) {
            content = bannerCard(Icons.INFO, null,
                GetText.tr("Turn packs on in Minecraft: Options, Resource Packs."), null);
        }
        if (content != null) {
            banner.add(content, BorderLayout.CENTER);
        }
        banner.revalidate();
        banner.repaint();
    }

    private interface BtnAction {
        void run(Btn button);
    }

    private static Btn button(String text, Icons icon, BtnAction action) {
        Btn b = new Btn(text, Btn.Kind.FILLED).icon(icon).compact();
        b.addActionListener(e -> action.run(b));
        return b;
    }

    private static JComponent bannerCard(Icons icon, String heading, String text, Btn action) {
        boolean strong = heading != null;
        RoundPanel card = new RoundPanel(new BorderLayout(12, 0), 18,
            strong ? Palette::accentSoft : Palette::card);
        if (!strong) {
            card.line(Palette::outline);
        }
        card.pad(10, 14, 10, 10);
        JLabel glyph = new JLabel(icon.asIcon(20, strong ? Palette::accent : Palette::textDim));
        card.add(glyph, BorderLayout.WEST);
        JPanel words = new JPanel();
        words.setOpaque(false);
        words.setLayout(new BoxLayout(words, BoxLayout.Y_AXIS));
        if (heading != null) {
            JLabel h = Lbl.text(heading, 13.5f, true, Palette::text);
            h.setAlignmentX(LEFT_ALIGNMENT);
            words.add(h);
        }
        JLabel t = Lbl.text(text, 12.5f, false, Palette::textDim);
        t.setAlignmentX(LEFT_ALIGNMENT);
        words.add(t);
        JPanel middle = new JPanel(new GridBagLayout());
        middle.setOpaque(false);
        GridBagConstraints c = new GridBagConstraints();
        c.weightx = 1;
        c.fill = GridBagConstraints.HORIZONTAL;
        middle.add(words, c);
        card.add(middle, BorderLayout.CENTER);
        if (action != null) {
            JPanel right = new JPanel(new GridBagLayout());
            right.setOpaque(false);
            right.add(action);
            card.add(right, BorderLayout.EAST);
        }
        if (!strong) {
            card.setPreferredSize(new Dimension(200, 42));
        }
        return card;
    }

    private void renderEmpty() {
        empty.removeAll();
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));

        boolean noLoader = kind == ContentKind.MODS && service.loaderKey() == null;
        String title;
        String text;
        switch (kind) {
            case RESOURCE_PACKS:
                title = GetText.tr("No resource packs yet");
                text = GetText.tr("Resource packs change how blocks, items and menus look. Popular for Minecraft {0}:",
                    service.minecraftVersion());
                break;
            case SHADERS:
                title = GetText.tr("No shaders yet");
                text = GetText.tr("Shaders add real looking light, shadows and water. Popular for Minecraft {0}:",
                    service.minecraftVersion());
                break;
            default:
                title = GetText.tr("No mods yet");
                text = noLoader
                    ? GetText.tr("Mods need a mod loader such as Fabric. Add one with the button above, then pick some mods.")
                    : GetText.tr("Mods add new things to Minecraft or make it run faster. Popular with {0} {1} players:",
                        LoaderSupport.displayName(service.loaderKey()), service.minecraftVersion());
                break;
        }

        Btn browse = new Btn(GetText.tr("Browse all {0}", noun()), Btn.Kind.FILLED).icon(Icons.SEARCH);
        browse.addActionListener(e -> showMode(BROWSE));
        JPanel head = ContentCards.message(ContentCards.iconFor(kind), title, text);
        head.setAlignmentX(CENTER_ALIGNMENT);
        column.add(head);

        if (!noLoader) {
            JPanel popular = new JPanel(new FlowLayout(FlowLayout.CENTER, 0, 0));
            popular.setOpaque(false);
            popular.setAlignmentX(CENTER_ALIGNMENT);
            JPanel spin = new JPanel(new GridBagLayout());
            spin.setOpaque(false);
            spin.setPreferredSize(new Dimension(200, 120));
            spin.add(new Spinner(32));
            popular.add(spin);
            column.add(popular);
            loadPopular(popular);

            column.add(Box.createVerticalStrut(18));
            JPanel buttons = new JPanel(new FlowLayout(FlowLayout.CENTER, 10, 0));
            buttons.setOpaque(false);
            buttons.add(browse);
            buttons.setAlignmentX(CENTER_ALIGNMENT);
            column.add(buttons);
            column.add(Box.createVerticalStrut(10));
            JLabel drop = Lbl.text(GetText.tr("or drop files from your computer anywhere in this window"), 12f,
                false, Palette::textDim);
            drop.setAlignmentX(CENTER_ALIGNMENT);
            column.add(drop);
        }

        empty.add(column, new GridBagConstraints());
        empty.revalidate();
        empty.repaint();
    }

    private void loadPopular(JPanel target) {
        owner.lookup(() -> service.popular(kind, 3), hits -> {
            target.removeAll();
            if (hits == null) {
                Btn retry = new Btn(GetText.tr("Try again"), Btn.Kind.TONAL).icon(Icons.REFRESH).compact();
                retry.addActionListener(e -> {
                    target.removeAll();
                    JPanel spin = new JPanel(new GridBagLayout());
                    spin.setOpaque(false);
                    spin.add(new Spinner(32));
                    target.add(spin);
                    target.revalidate();
                    loadPopular(target);
                });
                JPanel off = new JPanel();
                off.setOpaque(false);
                off.setLayout(new BoxLayout(off, BoxLayout.Y_AXIS));
                JLabel l = Lbl.dim(GetText.tr("Couldn't reach Modrinth. Check your connection."));
                l.setAlignmentX(CENTER_ALIGNMENT);
                retry.setAlignmentX(CENTER_ALIGNMENT);
                off.add(l);
                off.add(Box.createVerticalStrut(8));
                off.add(retry);
                target.add(off);
            } else {
                JPanel row = new JPanel(new FlowLayout(FlowLayout.CENTER, 14, 8));
                row.setOpaque(false);
                for (ModrinthSearchHit hit : hits) {
                    row.add(new ContentCards.ResultCard(owner, kind, hit));
                }
                target.add(row);
            }
            target.revalidate();
            target.repaint();
        });
    }

    // ---------------------------------------------------------------- browsing Modrinth

    private JComponent browseView() {
        JPanel view = new JPanel(new BorderLayout());
        view.setOpaque(false);
        browseBody.setOpaque(false);

        JPanel loading = new JPanel(new GridBagLayout());
        loading.setOpaque(false);
        loading.add(new Spinner(44));
        browseBody.add(loading, "loading");

        JPanel gridWrap = new JPanel(new BorderLayout(0, 8));
        gridWrap.setOpaque(false);
        gridWrap.setBorder(BorderFactory.createEmptyBorder(2, 0, 16, 0));
        gridWrap.add(grid, BorderLayout.NORTH);
        JPanel moreRow = new JPanel(new FlowLayout(FlowLayout.CENTER));
        moreRow.setOpaque(false);
        loadMore.addActionListener(e -> browse(false));
        moreRow.add(loadMore);
        gridWrap.add(moreRow, BorderLayout.CENTER);
        FitWidthPanel scrollContent = new FitWidthPanel(0);
        scrollContent.setLayout(new BorderLayout());
        scrollContent.add(gridWrap, BorderLayout.NORTH);
        JScrollPane scroll = FitWidthPanel.scroll(scrollContent);
        browseBody.add(scroll, "results");

        browseMessage.setOpaque(false);
        browseBody.add(browseMessage, "message");
        view.add(browseBody, BorderLayout.CENTER);
        return view;
    }

    private void showBrowseMessage(Icons icon, String heading, String text, JComponent... actions) {
        browseMessage.removeAll();
        browseMessage.add(ContentCards.message(icon, heading, text, actions), new GridBagConstraints());
        browseMessage.revalidate();
        browseMessage.repaint();
        browseCards.show(browseBody, "message");
    }

    /** Searches Modrinth. {@code fresh} starts again from the first page, otherwise the next page is added. */
    void browse(boolean fresh) {
        browsedOnce = true;
        if (kind == ContentKind.MODS && service.loaderKey() == null) {
            Btn add = new Btn(GetText.tr("Add Fabric"), Btn.Kind.FILLED).icon(Icons.PLUS);
            add.addActionListener(e -> owner.addFabric());
            showBrowseMessage(Icons.PUZZLE, GetText.tr("Add a mod loader first"),
                GetText.tr("This instance runs plain Minecraft. Mods need a loader such as Fabric."), add);
            return;
        }
        String query = search.getText().trim();
        if (query.length() == 1) {
            return;
        }
        if (fresh) {
            page = 0;
            browseCards.show(browseBody, "loading");
        } else {
            page++;
            loadMore.setEnabled(false);
            loadMore.setText(GetText.tr("Loading..."));
        }
        final int mine = ++ticket;
        final int wantedPage = page;
        @SuppressWarnings("unchecked")
        ComboItem<String> sortItem = (ComboItem<String>) sort.getSelectedItem();
        String sortBy = sortItem == null ? "relevance" : sortItem.getValue();
        owner.lookup(() -> service.search(kind, query, wantedPage, sortBy), result -> {
            if (mine != ticket) {
                return;
            }
            loadMore.setEnabled(true);
            loadMore.setText(GetText.tr("Show more"));
            showResults(result, fresh, query);
        });
    }

    private void showResults(ModrinthSearchResult result, boolean fresh, String query) {
        if (result == null) {
            if (!fresh) {
                page--;
                toastError(GetText.tr("Couldn't reach Modrinth. Check your connection."));
                return;
            }
            Btn retry = new Btn(GetText.tr("Try again"), Btn.Kind.TONAL).icon(Icons.REFRESH);
            retry.addActionListener(e -> browse(true));
            showBrowseMessage(Icons.WARNING, GetText.tr("Couldn't reach Modrinth"),
                GetText.tr("Check your internet connection and try again."), retry);
            return;
        }
        if (fresh) {
            grid.removeAll();
        }
        if (fresh && result.hits.isEmpty()) {
            showBrowseMessage(Icons.SEARCH, query.isEmpty() ? GetText.tr("Nothing here yet")
                : GetText.tr("No {0} found for \"{1}\"", noun(), query),
                GetText.tr("Only {0} made for Minecraft {1} are shown.", noun(), service.minecraftVersion()));
            return;
        }
        for (ModrinthSearchHit hit : result.hits) {
            grid.add(new ContentCards.ResultCard(owner, kind, hit));
        }
        loadMore.setVisible(result.offset + result.limit < result.totalHits);
        grid.revalidate();
        grid.repaint();
        browseCards.show(browseBody, "results");
        if (fresh) {
            SwingUtilities.invokeLater(() -> {
                Component scroll = browseBody.getComponent(1);
                if (scroll instanceof JScrollPane) {
                    ((JScrollPane) scroll).getVerticalScrollBar().setValue(0);
                }
            });
        }
    }

    private static void toastError(String text) {
        if (com.blockforge.launcher.App.TOASTER != null) {
            com.blockforge.launcher.App.TOASTER.popError(text);
        }
    }

    /** Sets the search text, for opening the manager straight on a search. */
    void setQuery(String query) {
        search.setText(query);
    }
}
