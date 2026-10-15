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
import java.awt.Component;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.function.Consumer;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.DefaultListModel;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JList;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.ListCellRenderer;
import javax.swing.ListSelectionModel;
import javax.swing.SwingUtilities;
import javax.swing.event.DocumentEvent;
import javax.swing.event.DocumentListener;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.data.MCVersionRow;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.data.minecraft.loaders.LoaderVersion;
import com.blockforge.launcher.evnt.listener.SettingsListener;
import com.blockforge.launcher.evnt.manager.SettingsManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.utils.ComboItem;
import com.blockforge.launcher.viewmodel.base.ICreatePackViewModel;
import com.blockforge.launcher.viewmodel.impl.CreatePackViewModel;

import io.reactivex.rxjava3.core.Observable;
import io.reactivex.rxjava3.disposables.CompositeDisposable;

/**
 * Build an instance (or a server) from any Minecraft version, with or without a mod loader.
 */
public class CreatePage extends Page {
    private static final long serialVersionUID = 1L;

    private final ICreatePackViewModel viewModel = new CreatePackViewModel();
    private final CompositeDisposable subscriptions = new CompositeDisposable();

    private final SearchBox nameField = new SearchBox(GetText.tr("Instance name"), false, 24);
    private final DefaultListModel<MCVersionRow> versionModel = new DefaultListModel<>();
    private final JList<MCVersionRow> versionList = new JList<>(versionModel);
    private final JComboBox<ComboItem<LoaderVersion>> loaderVersions = new JComboBox<>();
    private final Btn createInstance = new Btn(GetText.tr("Create instance"), Btn.Kind.FILLED).icon(Icons.CUBE);
    private final Btn createServer = new Btn(GetText.tr("Create server"), Btn.Kind.TONAL).icon(Icons.SERVER);

    /** set while the view model is the one changing a control, so the change isn't sent straight back */
    private boolean updating;
    private long lastLoaderVersionChange = System.currentTimeMillis();
    private boolean scrolledToSelection;

    public CreatePage() {
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        column.setBorder(BorderFactory.createEmptyBorder(28, 36, 36, 36));

        JLabel title = Lbl.title(GetText.tr("New instance"));
        JLabel sub = Lbl.dim(GetText.tr("Pick a Minecraft version and, if you like, a mod loader. The newest version is selected for you."));
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        column.add(title);
        column.add(sub);
        column.add(Box.createVerticalStrut(22));
        column.add(card(GetText.tr("Name"), nameCard()));
        column.add(Box.createVerticalStrut(16));
        column.add(card(GetText.tr("Minecraft version"), versionCard()));
        column.add(Box.createVerticalStrut(16));
        column.add(card(GetText.tr("Mod loader"), loaderCard()));
        column.add(Box.createVerticalStrut(22));
        column.add(actions());

        JPanel top = new JPanel(new BorderLayout());
        top.setOpaque(false);
        top.add(column, BorderLayout.NORTH);
        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(top, BorderLayout.CENTER);
        add(FitWidthPanel.scroll(fit), BorderLayout.CENTER);

        bind();
    }

    private RoundPanel card(String heading, JComponent... content) {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 12), 24, Palette::card);
        card.line(Palette::outline);
        card.pad(16, 22, 18, 22);
        card.setAlignmentX(LEFT_ALIGNMENT);
        card.setMaximumSize(new Dimension(760, 2000));
        JLabel h = Lbl.text(heading.toUpperCase(java.util.Locale.ROOT), 11f, true, Palette::accent);
        card.add(h, BorderLayout.NORTH);
        JPanel body = new JPanel();
        body.setOpaque(false);
        body.setLayout(new BoxLayout(body, BoxLayout.Y_AXIS));
        for (JComponent c : content) {
            c.setAlignmentX(LEFT_ALIGNMENT);
            body.add(c);
        }
        card.add(body, BorderLayout.CENTER);
        return card;
    }

    private JComponent nameCard() {
        nameField.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
        nameField.getDocument().addDocumentListener(new DocumentListener() {
            @Override
            public void insertUpdate(DocumentEvent e) {
                send();
            }

            @Override
            public void removeUpdate(DocumentEvent e) {
                send();
            }

            @Override
            public void changedUpdate(DocumentEvent e) {
                send();
            }

            private void send() {
                if (!updating) {
                    viewModel.setName(nameField.getText());
                }
            }
        });
        return nameField;
    }

    private final List<Chip> filterChips = new ArrayList<>();

    private JComponent versionCard() {
        JPanel filters = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 0));
        filters.setOpaque(false);
        if (viewModel.showReleaseOption()) {
            filters.add(filter(GetText.tr("Releases"), viewModel.releaseSelected(), viewModel.releaseEnabled(),
                viewModel::setReleaseSelected));
        }
        if (viewModel.showSnapshotOption()) {
            filters.add(filter(GetText.tr("Snapshots"), viewModel.snapshotSelected(), viewModel.snapshotEnabled(),
                viewModel::setSnapshotSelected));
        }
        if (viewModel.showExperimentOption()) {
            filters.add(filter(GetText.tr("Experiments"), viewModel.experimentSelected(),
                viewModel.experimentEnabled(), viewModel::setExperimentSelected));
        }
        if (viewModel.showOldBetaOption()) {
            filters.add(filter(GetText.tr("Betas"), viewModel.oldBetaSelected(), viewModel.oldBetaEnabled(),
                viewModel::setOldBetaSelected));
        }
        if (viewModel.showOldAlphaOption()) {
            filters.add(filter(GetText.tr("Alphas"), viewModel.oldAlphaSelected(), viewModel.oldAlphaEnabled(),
                viewModel::setOldAlphaSelected));
        }

        versionList.setSelectionMode(ListSelectionModel.SINGLE_SELECTION);
        versionList.setCellRenderer(new VersionRenderer());
        versionList.setFixedCellHeight(40);
        versionList.setOpaque(false);
        versionList.addListSelectionListener(e -> {
            if (!updating && !e.getValueIsAdjusting()) {
                MCVersionRow row = versionList.getSelectedValue();
                if (row != null) {
                    viewModel.setSelectedMinecraftVersion(row.id);
                }
            }
        });
        JScrollPane scroll = FitWidthPanel.scroll(versionList);
        scroll.setPreferredSize(new Dimension(600, 280));
        scroll.setMaximumSize(new Dimension(Integer.MAX_VALUE, 280));

        JPanel wrap = new JPanel(new BorderLayout(0, 12));
        wrap.setOpaque(false);
        wrap.add(filters, BorderLayout.NORTH);
        wrap.add(scroll, BorderLayout.CENTER);
        wrap.setMaximumSize(new Dimension(Integer.MAX_VALUE, 340));
        return wrap;
    }

    private Chip filter(String label, Observable<Boolean> selected, Observable<Boolean> enabled,
        Consumer<Boolean> setter) {
        Chip chip = new Chip(label);
        chip.onClick(() -> setter.accept(!chip.isSelected()));
        subscriptions.add(selected.subscribe(on -> ui(() -> chip.setSelected(Boolean.TRUE.equals(on)))));
        subscriptions.add(enabled.subscribe(on -> ui(() -> chip.setEnabled(Boolean.TRUE.equals(on)))));
        filterChips.add(chip);
        return chip;
    }

    private JComponent loaderCard() {
        JPanel chips = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 6));
        chips.setOpaque(false);

        chips.add(loader(GetText.tr("None"), null, viewModel.loaderTypeNoneSelected(),
            viewModel.loaderTypeNoneEnabled(), null, true));
        chips.add(loader("Fabric", LoaderType.FABRIC, viewModel.loaderTypeFabricSelected(),
            viewModel.loaderTypeFabricEnabled(), viewModel.isFabricVisible(), viewModel.showFabricOption()));
        chips.add(loader("Forge", LoaderType.FORGE, viewModel.loaderTypeForgeSelected(),
            viewModel.loaderTypeForgeEnabled(), viewModel.isForgeVisible(), viewModel.showForgeOption()));
        chips.add(loader("Legacy Fabric", LoaderType.LEGACY_FABRIC, viewModel.loaderTypeLegacyFabricSelected(),
            viewModel.loaderTypeLegacyFabricEnabled(), viewModel.isLegacyFabricVisible(),
            viewModel.showLegacyFabricOption()));
        chips.add(loader("NeoForge", LoaderType.NEOFORGE, viewModel.loaderTypeNeoForgeSelected(),
            viewModel.loaderTypeNeoForgeEnabled(), viewModel.isNeoForgeVisible(), viewModel.showNeoForgeOption()));
        chips.add(loader("Quilt", LoaderType.QUILT, viewModel.loaderTypeQuiltSelected(),
            viewModel.loaderTypeQuiltEnabled(), viewModel.isQuiltVisible(), viewModel.showQuiltOption()));
        chips.add(loader("Paper", LoaderType.PAPER, viewModel.loaderTypePaperSelected(),
            viewModel.loaderTypePaperEnabled(), viewModel.isPaperVisible(), viewModel.showPaperOption()));
        chips.add(loader("Purpur", LoaderType.PURPUR, viewModel.loaderTypePurpurSelected(),
            viewModel.loaderTypePurpurEnabled(), viewModel.isPurpurVisible(), viewModel.showPurpurOption()));

        loaderVersions.setPreferredSize(new Dimension(320, 40));
        loaderVersions.addActionListener(e -> {
            // a person can't change the version within 100ms of the view model doing it
            if (!updating && e.getWhen() > lastLoaderVersionChange + 100) {
                @SuppressWarnings("unchecked")
                ComboItem<LoaderVersion> item = (ComboItem<LoaderVersion>) loaderVersions.getSelectedItem();
                if (item != null && item.getValue() != null) {
                    viewModel.setLoaderVersion(item.getValue());
                }
            }
        });
        JPanel versionRow = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 6));
        versionRow.setOpaque(false);
        versionRow.add(Lbl.dim(GetText.tr("Loader version")));
        versionRow.add(loaderVersions);

        JPanel wrap = new JPanel(new BorderLayout());
        wrap.setOpaque(false);
        wrap.add(chips, BorderLayout.NORTH);
        wrap.add(versionRow, BorderLayout.CENTER);
        wrap.setMaximumSize(new Dimension(Integer.MAX_VALUE, 140));
        return wrap;
    }

    private Chip loader(String label, LoaderType type, Observable<Boolean> selected, Observable<Boolean> enabled,
        Observable<Boolean> visible, boolean offered) {
        Chip chip = new Chip(label);
        chip.setVisible(offered);
        chip.onClick(() -> viewModel.setLoaderType(type));
        subscriptions.add(selected.subscribe(on -> ui(() -> chip.setSelected(Boolean.TRUE.equals(on)))));
        subscriptions.add(enabled.subscribe(on -> ui(() -> chip.setEnabled(Boolean.TRUE.equals(on)))));
        if (visible != null) {
            subscriptions.add(visible.subscribe(on -> ui(() -> chip.setVisible(offered && Boolean.TRUE.equals(on)))));
        }
        return chip;
    }

    private JComponent actions() {
        JPanel row = new JPanel(new FlowLayout(FlowLayout.LEFT, 10, 0));
        row.setOpaque(false);
        row.setAlignmentX(LEFT_ALIGNMENT);
        row.setMaximumSize(new Dimension(760, 44));
        createInstance.addActionListener(e -> viewModel.createInstance());
        createServer.addActionListener(e -> {
            if (viewModel.warnUserAboutServer()) {
                int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Are you sure you want to create a server?"))
                    .setContent(new HTMLBuilder().center().text(GetText.tr(
                        "Creating a server won't allow you play Minecraft, it's for letting others play together.<br/><br/>If you just want to play Minecraft, you don't want to create a server, and instead will want to create an instance.<br/><br/>Are you sure you want to create a server?"))
                        .build())
                    .setType(DialogManager.QUESTION).show();
                if (ret != 0) {
                    return;
                }
            }
            viewModel.createServer();
        });
        row.add(createInstance);
        row.add(createServer);
        return row;
    }

    private void bind() {
        subscriptions.add(viewModel.name().subscribe(name -> ui(() -> {
            String text = name.orElse("");
            if (!text.equals(nameField.getText())) {
                updating = true;
                int caret = Math.min(nameField.getCaretPosition(), text.length());
                nameField.setText(text);
                nameField.setCaretPosition(caret);
                updating = false;
            }
        })));

        subscriptions.add(viewModel.minecraftVersions().subscribe(rows -> ui(() -> {
            updating = true;
            versionModel.clear();
            rows.forEach(versionModel::addElement);
            updating = false;
        })));
        subscriptions.add(viewModel.selectedMinecraftVersionIndex().subscribe(index -> ui(() -> {
            if (index >= 0 && index < versionModel.size()) {
                updating = true;
                versionList.setSelectedIndex(index);
                if (!scrolledToSelection) {
                    versionList.ensureIndexIsVisible(index);
                    scrolledToSelection = true;
                }
                updating = false;
            }
        })));

        subscriptions.add(viewModel.loaderVersionsDropDownEnabled()
            .subscribe(on -> ui(() -> loaderVersions.setEnabled(Boolean.TRUE.equals(on)))));
        subscriptions.add(viewModel.loaderVersions().subscribe(versions -> ui(() -> {
            updating = true;
            loaderVersions.removeAllItems();
            Optional<List<LoaderVersion>> list = versions;
            if (!list.isPresent()) {
                loaderVersions.addItem(new ComboItem<>(null, GetText.tr("Select a loader first")));
            } else {
                for (LoaderVersion version : list.get()) {
                    loaderVersions.addItem(new ComboItem<>(version, version.toString()));
                }
            }
            updating = false;
        })));
        subscriptions.add(viewModel.selectedLoaderVersionIndex().subscribe(index -> ui(() -> {
            if (index >= 0 && index < loaderVersions.getItemCount()) {
                updating = true;
                lastLoaderVersionChange = System.currentTimeMillis();
                loaderVersions.setSelectedIndex(index);
                updating = false;
            }
        })));
        subscriptions.add(viewModel.loaderLoading().subscribe(loading -> ui(() -> {
            if (Boolean.TRUE.equals(loading)) {
                updating = true;
                loaderVersions.removeAllItems();
                loaderVersions.addItem(new ComboItem<>(null, GetText.tr("Getting loader versions")));
                updating = false;
            }
        })));

        subscriptions.add(viewModel.createInstanceEnabled()
            .subscribe(on -> ui(() -> createInstance.setEnabled(Boolean.TRUE.equals(on)))));
        subscriptions.add(viewModel.createServerEnabled()
            .subscribe(on -> ui(() -> createServer.setEnabled(Boolean.TRUE.equals(on)))));
        subscriptions.add(viewModel.createInstanceDisabledReason()
            .subscribe(reason -> ui(() -> createInstance.setToolTipText(reason.orElse(null)))));
    }

    private static void ui(Runnable run) {
        if (SwingUtilities.isEventDispatchThread()) {
            run.run();
        } else {
            SwingUtilities.invokeLater(run);
        }
    }

    @Override
    public void onShow() {
        scrolledToSelection = false;
    }

    @Override
    public void dispose() {
        subscriptions.dispose();
        if (viewModel instanceof SettingsListener) {
            SettingsManager.removeListener((SettingsListener) viewModel);
        }
    }

    /** One row of the Minecraft version list. */
    private static final class VersionRenderer implements ListCellRenderer<MCVersionRow> {
        @Override
        public Component getListCellRendererComponent(JList<? extends MCVersionRow> list, MCVersionRow row,
            int index, boolean selected, boolean focus) {
            return new JPanel() {
                private static final long serialVersionUID = 1L;

                @Override
                public Dimension getPreferredSize() {
                    return new Dimension(200, 40);
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
                        g.fillRoundRect(0, 3, w, h - 6, 16, 16);
                    }
                    g.setFont(Palette.bold(14f));
                    java.awt.FontMetrics fm = g.getFontMetrics();
                    int baseline = (h - fm.getHeight()) / 2 + fm.getAscent();
                    g.setColor(selected ? Palette.accent() : Palette.text());
                    g.drawString(row.id, 14, baseline);
                    g.setFont(Palette.font(12f));
                    fm = g.getFontMetrics();
                    g.setColor(Palette.textDim());
                    String right = row.type + "  ·  " + row.date;
                    g.drawString(right, w - 14 - fm.stringWidth(right), baseline);
                    g.dispose();
                }
            };
        }
    }
}
