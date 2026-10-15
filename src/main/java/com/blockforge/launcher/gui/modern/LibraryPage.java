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
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.managers.InstanceManager;

import io.reactivex.rxjava3.disposables.Disposable;

/**
 * All of the user's instances as cards, with search and sorting.
 */
public class LibraryPage extends Page {
    private static final long serialVersionUID = 1L;

    private final Map<Instance, InstanceTile> tiles = new HashMap<>();
    private final FitWidthPanel grid = new FitWidthPanel(18);
    private final JPanel emptyState = new JPanel(new BorderLayout());
    private final JLabel countLabel = Lbl.dim("");
    private final SearchBox search = new SearchBox(GetText.tr("Search your instances"), true, 24);
    private final ChipRow sortChips;
    private final CardLayout cards = new CardLayout();
    private final JPanel center = new JPanel(cards);
    private List<Instance> instances = new ArrayList<>();
    private Disposable subscription;

    public LibraryPage() {
        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        header.setBorder(BorderFactory.createEmptyBorder(28, 36, 12, 36));

        JPanel titleRow = new JPanel(new BorderLayout());
        titleRow.setOpaque(false);
        JPanel titles = new JPanel();
        titles.setOpaque(false);
        titles.setLayout(new BoxLayout(titles, BoxLayout.Y_AXIS));
        JLabel title = Lbl.title(GetText.tr("Library"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        countLabel.setAlignmentX(LEFT_ALIGNMENT);
        titles.add(title);
        titles.add(countLabel);
        titleRow.add(titles, BorderLayout.WEST);

        JPanel actions = FitWidthPanel.rightButtons();
        Btn importButton = new Btn(GetText.tr("Import"), Btn.Kind.TONAL).icon(Icons.IMPORT);
        importButton.addActionListener(e -> ImportWizardDialog.open(App.launcherFrame));
        Btn newButton = new Btn(GetText.tr("New instance"), Btn.Kind.FILLED).icon(Icons.PLUS);
        newButton.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_PACKS_TAB));
        actions.add(importButton);
        actions.add(newButton);
        titleRow.add(actions, BorderLayout.EAST);
        header.add(titleRow);
        header.add(Box.createVerticalStrut(18));

        JPanel filterRow = new JPanel(new BorderLayout(16, 0));
        filterRow.setOpaque(false);
        search.setPreferredSize(new Dimension(340, 44));
        search.onChange(150, this::rebuild);
        JPanel searchWrap = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        searchWrap.setOpaque(false);
        searchWrap.add(search);
        filterRow.add(searchWrap, BorderLayout.WEST);

        sortChips = new ChipRow(new String[] { GetText.tr("Recent"), GetText.tr("A–Z"), GetText.tr("Most played") },
            0, i -> rebuild());
        JPanel chipWrap = new JPanel(new FlowLayout(FlowLayout.RIGHT, 0, 4));
        chipWrap.setOpaque(false);
        chipWrap.add(sortChips);
        filterRow.add(chipWrap, BorderLayout.EAST);
        header.add(filterRow);
        add(header, BorderLayout.NORTH);

        JPanel gridWrap = new JPanel(new BorderLayout());
        gridWrap.setOpaque(false);
        gridWrap.setBorder(BorderFactory.createEmptyBorder(8, 36, 36, 36));
        gridWrap.add(grid, BorderLayout.NORTH);
        center.setOpaque(false);
        javax.swing.JScrollPane gridScroll = FitWidthPanel.scroll(wrapScrollable(gridWrap));
        FitWidthPanel.alignHeader(header, gridScroll, 28, 36, 12);
        center.add(gridScroll, "grid");


        JPanel none = new JPanel(new java.awt.GridBagLayout());
        none.setOpaque(false);
        none.add(Lbl.dim(GetText.tr("No instances match your search")));
        center.add(none, "none");
        add(center, BorderLayout.CENTER);

        buildEmptyState();
    }

    /** the grid wrapper has to track the viewport width too, or wrapping never kicks in */
    private static JPanel wrapScrollable(JPanel inner) {
        FitWidthPanel outer = new FitWidthPanel(0);
        outer.setLayout(new BorderLayout());
        outer.add(inner, BorderLayout.CENTER);
        return outer;
    }

    private void buildEmptyState() {
        emptyState.setOpaque(false);
        RoundPanel card = new RoundPanel(new BoxLayout(new JPanel(), BoxLayout.Y_AXIS), 28, Palette::card);
        card.setLayout(new BoxLayout(card, BoxLayout.Y_AXIS));
        card.line(Palette::outline);
        card.pad(40, 56, 40, 56);

        JLabel icon = new JLabel(Icons.CUBE.asIcon(56, Palette::accent));
        icon.setAlignmentX(Component.CENTER_ALIGNMENT);
        JLabel heading = Lbl.heading(GetText.tr("No instances yet"));
        heading.setAlignmentX(Component.CENTER_ALIGNMENT);
        JLabel sub = Lbl.dim(GetText.tr("Install a modpack or create your own to start playing."));
        sub.setAlignmentX(Component.CENTER_ALIGNMENT);
        Btn browse = new Btn(GetText.tr("Browse modpacks"), Btn.Kind.FILLED).icon(Icons.EXPLORE);
        browse.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_PACKS_TAB));
        Btn create = new Btn(GetText.tr("Create custom instance"), Btn.Kind.TONAL).icon(Icons.CUBE);
        create.addActionListener(e -> App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB));
        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.CENTER, 10, 0));
        buttons.setOpaque(false);
        buttons.add(browse);
        buttons.add(create);

        card.add(icon);
        card.add(Box.createVerticalStrut(16));
        card.add(heading);
        card.add(Box.createVerticalStrut(6));
        card.add(sub);
        card.add(Box.createVerticalStrut(22));
        card.add(buttons);

        JPanel holder = new JPanel(new java.awt.GridBagLayout());
        holder.setOpaque(false);
        holder.add(card);
        emptyState.add(holder, BorderLayout.CENTER);
        center.add(emptyState, "empty");
    }

    @Override
    public void addNotify() {
        super.addNotify();
        if (subscription == null) {
            subscription = InstanceManager.getInstancesObservable().subscribe(list -> SwingUtilities.invokeLater(() -> {
                instances = new ArrayList<>(list);
                rebuild();
            }));
        }
    }

    @Override
    public void removeNotify() {
        if (subscription != null) {
            subscription.dispose();
            subscription = null;
        }
        super.removeNotify();
    }

    @Override
    public void onShow() {
        // last played times may have changed
        tiles.values().forEach(InstanceTile::refresh);
        rebuild();
    }

    private Comparator<Instance> comparator() {
        switch (sortChips.getSelected()) {
            case 1:
                return Comparator.comparing(i -> i.launcher.name.toLowerCase(Locale.ROOT));
            case 2:
                return Comparator.comparingLong(Instance::getNumberOfPlays).reversed();
            case 0:
            default:
                return Comparator.comparing(Instance::getLastPlayedOrEpoch).reversed();
        }
    }

    private void rebuild() {
        String query = search.getText().trim().toLowerCase(Locale.ROOT);
        List<Instance> shown = new ArrayList<>();
        for (Instance instance : instances) {
            if (query.isEmpty() || instance.launcher.name.toLowerCase(Locale.ROOT).contains(query)
                || (instance.launcher.pack != null && instance.launcher.pack.toLowerCase(Locale.ROOT).contains(query))) {
                shown.add(instance);
            }
        }
        shown.sort(comparator());

        countLabel.setText(instances.size() == 1 ? GetText.tr("1 instance")
            : GetText.tr("{0} instances", instances.size()));

        grid.removeAll();
        tiles.keySet().retainAll(instances);
        for (Instance instance : shown) {
            InstanceTile tile = tiles.computeIfAbsent(instance, i -> {
                InstanceTile[] self = new InstanceTile[1];
                self[0] = new InstanceTile(i, () -> {
                    self[0].refresh();
                    rebuild();
                });
                return self[0];
            });
            grid.add(tile);
        }

        cards.show(center, instances.isEmpty() ? "empty" : shown.isEmpty() ? "none" : "grid");
        grid.revalidate();
        grid.repaint();
    }
}
