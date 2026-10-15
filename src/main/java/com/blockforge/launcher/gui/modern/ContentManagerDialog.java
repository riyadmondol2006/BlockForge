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
import java.awt.Color;
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.Window;
import java.awt.image.BufferedImage;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.EnumMap;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.WeakHashMap;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.function.Consumer;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JFileChooser;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.KeyStroke;
import javax.swing.SwingUtilities;
import javax.swing.WindowConstants;
import javax.swing.filechooser.FileNameExtensionFilter;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.content.ContentKind;
import com.blockforge.launcher.content.ContentService;
import com.blockforge.launcher.content.LoaderSupport;
import com.blockforge.launcher.content.PerformancePack;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.gui.dialogs.AddModsDialog;
import com.blockforge.launcher.gui.dialogs.ModrinthVersionSelectorDialog;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.OS;

/**
 * One place to see and change an instance's mods, resource packs and shaders, and to find more on Modrinth with a
 * single click per item. Files can be dropped anywhere on the window and land in the right folder.
 */
public final class ContentManagerDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private final Instance instance;
    private final ContentService service;
    private final Runnable changed;

    /** installs, updates and file changes, one at a time so they never trip over each other */
    private final ExecutorService worker = Executors.newSingleThreadExecutor(r -> daemon(r, "content-changes"));
    /** searches and look-ups, which mustn't wait behind a big download */
    private final ExecutorService lookups = Executors.newFixedThreadPool(2, r -> daemon(r, "content-lookups"));

    private final TabStrip tabs;
    private final CardLayout tabCards = new CardLayout();
    private final JPanel tabBody = new JPanel(tabCards);
    private final Map<ContentKind, ContentPanel> panels = new EnumMap<>(ContentKind.class);
    private WindowFileDrop fileDrop;

    /** every result card on screen, so all copies of a project show the same install progress */
    private final Set<ContentCards.ResultCard> cards = Collections
        .newSetFromMap(new WeakHashMap<ContentCards.ResultCard, Boolean>());
    private final Map<String, InstallButton.State> states = new HashMap<>();
    private final Map<String, Double> progress = new HashMap<>();
    private Set<String> installedIds = Collections.emptySet();
    private boolean busyPerformance;

    private static Thread daemon(Runnable r, String name) {
        Thread t = new Thread(r, name);
        t.setDaemon(true);
        return t;
    }

    private ContentManagerDialog(Window owner, Instance instance, ContentKind start, Runnable changed) {
        super(owner, GetText.tr("Mods and packs for {0}", instance.launcher.name), Dialog.ModalityType.DOCUMENT_MODAL);
        this.instance = instance;
        this.service = new ContentService(instance);
        this.changed = changed == null ? () -> {
        } : changed;
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
        setIconImage(com.blockforge.launcher.utils.Utils.getImage("/assets/image/icon.png"));

        JPanel root = new JPanel(new BorderLayout(0, 16)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(22, 28, 22, 28));

        ContentKind[] kinds = ContentKind.values();
        String[] names = new String[kinds.length];
        Icons[] icons = new Icons[kinds.length];
        for (int i = 0; i < kinds.length; i++) {
            names[i] = title(kinds[i]);
            icons[i] = ContentCards.iconFor(kinds[i]);
        }
        tabs = new TabStrip(names, icons, i -> tabCards.show(tabBody, kinds[i].name()));

        JPanel north = new JPanel(new BorderLayout(0, 14));
        north.setOpaque(false);
        north.add(header(), BorderLayout.NORTH);
        JPanel tabRow = new JPanel(new BorderLayout());
        tabRow.setOpaque(false);
        tabRow.add(tabs, BorderLayout.WEST);
        JComponent rule = new JComponent() {
            private static final long serialVersionUID = 1L;

            @Override
            protected void paintComponent(Graphics g) {
                g.setColor(Palette.outline());
                g.fillRect(0, getHeight() - 1, getWidth(), 1);
            }
        };
        rule.setPreferredSize(new Dimension(10, 1));
        tabRow.add(rule, BorderLayout.SOUTH);
        north.add(tabRow, BorderLayout.CENTER);
        root.add(north, BorderLayout.NORTH);

        tabBody.setOpaque(false);
        for (ContentKind kind : kinds) {
            ContentPanel panel = new ContentPanel(this, kind);
            panels.put(kind, panel);
            tabBody.add(panel, kind.name());
            panel.showLoadingInstalled();
        }
        root.add(tabBody, BorderLayout.CENTER);

        setContentPane(root);
        fileDrop = WindowFileDrop.install(this, instance, i -> {
            refreshAll();
            checkUpdates();
        });

        getRootPane().registerKeyboardAction(e -> dispose(), KeyStroke.getKeyStroke("ESCAPE"),
            JComponent.WHEN_IN_FOCUSED_WINDOW);

        setPreferredSize(new Dimension(1080, 740));
        setMinimumSize(new Dimension(860, 560));
        pack();
        setLocationRelativeTo(owner);

        tabs.select(start == null ? 0 : start.ordinal());
        tabCards.show(tabBody, (start == null ? ContentKind.MODS : start).name());

        // files put in the folders by hand show up too
        worker.execute(() -> {
            boolean found = false;
            try {
                found = service.scanUntracked();
            } catch (RuntimeException e) {
                LogManager.logStackTrace("Couldn't look for new files", e);
            }
            boolean anyFound = found;
            SwingUtilities.invokeLater(() -> {
                refreshAll();
                checkUpdates();
                if (anyFound) {
                    this.changed.run();
                }
            });
        });
    }

    /**
     * Opens the manager for an instance.
     *
     * @param start   the tab to show first
     * @param changed runs when the window closes, to refresh whatever shows the instance
     */
    public static void open(Window owner, Instance instance, ContentKind start, Runnable changed) {
        ContentManagerDialog dialog = new ContentManagerDialog(owner == null ? App.launcherFrame : owner, instance,
            start, changed);
        dialog.setVisible(true);
    }

    /** Builds the window without showing it, for screenshots and tests. */
    static ContentManagerDialog create(Window owner, Instance instance, ContentKind start) {
        return new ContentManagerDialog(owner, instance, start, null);
    }

    static String title(ContentKind kind) {
        switch (kind) {
            case RESOURCE_PACKS:
                return GetText.tr("Resource packs");
            case SHADERS:
                return GetText.tr("Shaders");
            default:
                return GetText.tr("Mods");
        }
    }

    ContentService service() {
        return service;
    }

    ContentPanel panel(ContentKind kind) {
        return panels.get(kind);
    }

    void selectTab(ContentKind kind) {
        tabs.select(kind.ordinal());
        tabCards.show(tabBody, kind.name());
    }

    @Override
    public void dispose() {
        // installs that already started finish in the background; nothing new starts
        worker.shutdown();
        lookups.shutdownNow();
        super.dispose();
        changed.run();
    }

    // ---------------------------------------------------------------- header

    private JComponent header() {
        JPanel row = new JPanel(new BorderLayout(16, 0));
        row.setOpaque(false);

        ContentCards.Picture picture = new ContentCards.Picture(52, Icons.CUBE);
        java.io.File custom = instance.getRoot().resolve("instance.png").toFile();
        ImageCache.async(() -> {
            BufferedImage source = ImageCache.toBuffered(instance.getImage());
            return source == null ? null : ImageCache.cover(source, 104, 104);
        }, "instance-square:" + instance.getRoot() + "#" + custom.lastModified(), picture::setImage);
        row.add(picture, BorderLayout.WEST);

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel name = Lbl.text(instance.launcher.name, 21f, true, Palette::text);
        StringBuilder meta = new StringBuilder("Minecraft ").append(service.minecraftVersion());
        if (service.loaderKey() != null) {
            meta.append("  ·  ").append(LoaderSupport.displayName(service.loaderKey()));
        } else {
            meta.append("  ·  ").append(GetText.tr("no mod loader"));
        }
        JLabel sub = Lbl.text(meta.toString(), 13f, false, Palette::textDim);
        name.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        text.add(Box.createVerticalGlue());
        text.add(name);
        text.add(Box.createVerticalStrut(2));
        text.add(sub);
        text.add(Box.createVerticalGlue());
        row.add(text, BorderLayout.CENTER);

        JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 6));
        actions.setOpaque(false);
        Btn folder = new Btn(GetText.tr("Open folder"), Btn.Kind.TONAL).icon(Icons.FOLDER);
        folder.addActionListener(e -> {
            ContentKind kind = ContentKind.values()[tabs.getSelected()];
            java.nio.file.Path path = instance.getRoot().resolve(kind.folder);
            try {
                java.nio.file.Files.createDirectories(path);
            } catch (IOException ex) {
                path = instance.getRoot();
            }
            OS.openFileExplorer(path);
        });
        Btn done = new Btn(GetText.tr("Done"), Btn.Kind.FILLED);
        done.addActionListener(e -> dispose());
        actions.add(folder);
        actions.add(done);
        row.add(actions, BorderLayout.EAST);
        return row;
    }

    // ---------------------------------------------------------------- keeping everything in step

    /** Reads the instance again and redraws every tab. */
    void refreshAll() {
        installedIds = service.installedProjectIds();
        ContentKind[] kinds = ContentKind.values();
        for (int i = 0; i < kinds.length; i++) {
            ContentPanel panel = panels.get(kinds[i]);
            panel.refreshInstalled();
            tabs.setCount(i, panel.count() == 0 ? null : String.valueOf(panel.count()));
        }
        for (ContentCards.ResultCard card : new ArrayList<>(cards)) {
            apply(card);
        }
    }

    private void checkUpdates() {
        for (ContentKind kind : ContentKind.values()) {
            checkUpdates(kind);
        }
    }

    private void checkUpdates(ContentKind kind) {
        lookup(() -> service.findUpdates(kind), found -> {
            if (found != null) {
                panels.get(kind).setUpdates(found);
            }
        });
    }

    /** Runs a look-up off the interface thread and hands the result back on it. */
    <T> void lookup(Callable<T> task, Consumer<T> done) {
        if (lookups.isShutdown()) {
            return;
        }
        lookups.execute(() -> {
            T result = null;
            try {
                result = task.call();
            } catch (Exception e) {
                LogManager.debug("Content look-up failed: " + e.getMessage());
            }
            T finalResult = result;
            SwingUtilities.invokeLater(() -> {
                if (isDisplayable()) {
                    done.accept(finalResult);
                }
            });
        });
    }

    void register(ContentCards.ResultCard card) {
        cards.add(card);
        apply(card);
    }

    private void apply(ContentCards.ResultCard card) {
        InstallButton.State state = states.get(card.projectId);
        if (state == null || state == InstallButton.State.INSTALLED) {
            state = installedIds.contains(card.projectId) ? InstallButton.State.INSTALLED : InstallButton.State.READY;
        }
        if (card.install.getState() != state) {
            card.install.setState(state);
        }
        if (state == InstallButton.State.WORKING) {
            Double f = progress.get(card.projectId);
            card.install.setProgress(f == null ? -1 : f, null);
        }
    }

    private void setState(String projectId, InstallButton.State state) {
        states.put(projectId, state);
        if (state != InstallButton.State.WORKING) {
            progress.remove(projectId);
        }
        for (ContentCards.ResultCard card : new ArrayList<>(cards)) {
            if (card.projectId.equals(projectId)) {
                apply(card);
            }
        }
    }

    /** Progress reports from a background install, passed to the cards on the interface thread. */
    private ContentService.Progress progressFor(String projectId) {
        return new ContentService.Progress() {
            @Override
            public void status(String text) {
            }

            @Override
            public void fraction(double fraction) {
                SwingUtilities.invokeLater(() -> {
                    progress.put(projectId, fraction);
                    for (ContentCards.ResultCard card : new ArrayList<>(cards)) {
                        if (card.projectId.equals(projectId)) {
                            card.install.setProgress(fraction, null);
                        }
                    }
                });
            }
        };
    }

    /** Progress shown as the text of a button: "Adding Sodium... 40%". */
    private static ContentService.Progress progressOn(Btn button, String prefix) {
        return new ContentService.Progress() {
            private String status = "";

            @Override
            public void status(String text) {
                status = text;
                SwingUtilities.invokeLater(() -> {
                    button.setText(prefix + " " + status);
                    button.revalidate();
                });
            }

            @Override
            public void fraction(double fraction) {
                if (fraction >= 0) {
                    SwingUtilities.invokeLater(() -> {
                        button.setText(prefix + " " + status + " " + Math.round(fraction * 100) + "%");
                        button.revalidate();
                    });
                }
            }
        };
    }

    // ---------------------------------------------------------------- installing from Modrinth

    /** The Install button on a card: sorts out a shader loader if needed, then installs in the background. */
    void requestInstall(String projectId, String title, ContentKind kind) {
        boolean withShaderLoader = false;
        if (kind == ContentKind.SHADERS) {
            switch (service.shaderSetup()) {
                case ADD_IRIS: {
                    int answer = ChoiceDialog.ask(this, Icons.SPARKLE, GetText.tr("Shaders need Iris"),
                        GetText.tr(
                            "Minecraft can't show shaders on its own. Iris adds them, and Sodium keeps the game fast. Add both together with {0}?",
                            title),
                        GetText.tr("Shader only"), GetText.tr("Add Iris and Sodium too"));
                    if (answer < 0) {
                        return;
                    }
                    withShaderLoader = answer == 1;
                    break;
                }
                case ADD_OCULUS: {
                    int answer = ChoiceDialog.ask(this, Icons.SPARKLE, GetText.tr("Shaders need Oculus"),
                        GetText.tr(
                            "On Forge, shaders need Oculus (or OptiFine from optifine.net). Add Oculus from Modrinth together with {0}?",
                            title),
                        GetText.tr("Shader only"), GetText.tr("Add Oculus too"));
                    if (answer < 0) {
                        return;
                    }
                    withShaderLoader = answer == 1;
                    break;
                }
                case NEEDS_MOD_LOADER: {
                    int answer = ChoiceDialog.ask(this, Icons.WARNING, GetText.tr("Shaders need a mod loader"),
                        GetText.tr(
                            "This instance runs plain Minecraft, which can't show shaders. They work once the instance has Fabric and Iris. Download {0} anyway?",
                            title),
                        GetText.tr("Cancel"), GetText.tr("Download anyway"));
                    if (answer != 1) {
                        return;
                    }
                    break;
                }
                default:
                    break;
            }
        }

        boolean loaderToo = withShaderLoader;
        setState(projectId, InstallButton.State.QUEUED);
        if (worker.isShutdown()) {
            return;
        }
        worker.execute(() -> {
            SwingUtilities.invokeLater(() -> setState(projectId, InstallButton.State.WORKING));
            ContentService.Progress p = progressFor(projectId);
            String loaderWarning = null;
            try {
                if (loaderToo) {
                    try {
                        service.installShaderLoader(p);
                    } catch (IOException e) {
                        loaderWarning = e.getMessage();
                    }
                }
                ContentService.InstallResult result = service.install(projectId, kind, p);
                String warning = loaderWarning;
                SwingUtilities.invokeLater(() -> {
                    setState(projectId, InstallButton.State.INSTALLED);
                    refreshAll();
                    checkUpdates(kind);
                    if (result.installed.size() > 1 && App.TOASTER != null) {
                        App.TOASTER.pop(GetText.tr("Added {0}", String.join(", ", result.installed)));
                    }
                    if (warning != null) {
                        ChoiceDialog.ask(this, Icons.WARNING, GetText.tr("The shader was added"), warning
                            + "\n\n" + GetText.tr("On Forge, OptiFine from optifine.net can show shaders instead."),
                            GetText.tr("OK"));
                    }
                });
            } catch (IOException | RuntimeException e) {
                LogManager.warn("Couldn't install " + title + ": " + e.getMessage());
                SwingUtilities.invokeLater(() -> {
                    setState(projectId, InstallButton.State.FAILED);
                    refreshAll();
                    if (e instanceof ContentService.NoCompatibleVersionException) {
                        ChoiceDialog.ask(this, Icons.INFO, GetText.tr("Not available for this instance"),
                            e.getMessage(), GetText.tr("OK"));
                    } else if (App.TOASTER != null) {
                        App.TOASTER.popError(GetText.tr("Couldn't install {0}", title));
                    }
                });
            }
        });
    }

    /** Opens the full version list for a project, for when the newest isn't wanted. */
    void openVersionPicker(String projectId, ContentKind kind) {
        lookup(() -> service.project(projectId), project -> {
            if (project == null) {
                if (App.TOASTER != null) {
                    App.TOASTER.popError(GetText.tr("Couldn't reach Modrinth. Check your connection."));
                }
                return;
            }
            new ModrinthVersionSelectorDialog(this, project, instance).setVisible(true);
            refreshAll();
            checkUpdates(kind);
        });
    }

    /** CurseForge, data packs and worlds are still handled by the classic window. */
    void openClassicBrowser() {
        new AddModsDialog(this, instance).setVisible(true);
        worker.execute(() -> {
            service.scanUntracked();
            SwingUtilities.invokeLater(this::refreshAll);
        });
    }

    void addFabric() {
        instance.addLoader(LoaderType.FABRIC);
        // the loader decides what can be searched and installed, so start again
        if (service.loaderKey() != null) {
            ContentKind current = ContentKind.values()[tabs.getSelected()];
            dispose();
            open(getOwner(), instance, current, null);
        }
    }

    void runShaderLoader(Btn button) {
        button.setEnabled(false);
        worker.execute(() -> {
            try {
                service.installShaderLoader(progressOn(button, GetText.tr("Adding")));
                SwingUtilities.invokeLater(() -> {
                    refreshAll();
                    if (App.TOASTER != null) {
                        App.TOASTER.pop(GetText.tr("Shaders are ready"));
                    }
                });
            } catch (IOException | RuntimeException e) {
                SwingUtilities.invokeLater(() -> {
                    button.setEnabled(true);
                    refreshAll();
                    ChoiceDialog.ask(this, Icons.INFO, GetText.tr("Couldn't add a shader loader"),
                        e.getMessage() + (LoaderSupport.FORGE.equals(service.loaderKey())
                            ? "\n\n" + GetText.tr("On Forge, OptiFine from optifine.net can show shaders instead.")
                            : ""),
                        GetText.tr("OK"));
                });
            }
        });
    }

    void runPerformancePack(Btn button) {
        if (busyPerformance) {
            return;
        }
        int answer = ChoiceDialog.ask(this, Icons.BOLT, GetText.tr("Make Minecraft run faster"),
            GetText.tr(
                "Adds {0} from Modrinth. They make the game smoother and load faster, especially on older computers, and don't change how it plays. Anything not made for Minecraft {1} yet is skipped.",
                PerformancePack.describe(service.loaderKey()), service.minecraftVersion()),
            GetText.tr("Cancel"), GetText.tr("Add them"));
        if (answer != 1) {
            return;
        }
        busyPerformance = true;
        button.setEnabled(false);
        String label = button.getText();
        worker.execute(() -> {
            ContentService.InstallResult result = service
                .installPerformancePack(progressOn(button, GetText.tr("Adding")));
            SwingUtilities.invokeLater(() -> {
                busyPerformance = false;
                button.setEnabled(true);
                button.setText(label);
                refreshAll();
                checkUpdates(ContentKind.MODS);
                if (App.TOASTER != null) {
                    if (result.installed.isEmpty()) {
                        App.TOASTER.pop(result.skipped.isEmpty() ? GetText.tr("Performance mods are already installed")
                            : GetText.tr("No performance mods are available for Minecraft {0} yet",
                                service.minecraftVersion()));
                    } else {
                        App.TOASTER.pop(GetText.tr("Added {0}", String.join(", ", result.installed)));
                    }
                }
            });
        });
    }

    // ---------------------------------------------------------------- installed items

    void setEnabled(ContentCards.Row row, boolean on) {
        row.toggle.setEnabled(false);
        worker.execute(() -> {
            service.setEnabled(row.mod, on);
            SwingUtilities.invokeLater(() -> {
                row.toggle.setEnabled(true);
                refreshAll();
            });
        });
    }

    void remove(ContentCards.Row row) {
        String name = ContentService.nameOf(row.mod);
        int answer = ChoiceDialog.ask(this, Icons.TRASH, GetText.tr("Remove {0}?", name),
            GetText.tr("The file is deleted from this instance. You can add it again at any time."),
            GetText.tr("Cancel"), GetText.tr("Remove"));
        if (answer != 1) {
            return;
        }
        worker.execute(() -> {
            service.remove(row.mod);
            SwingUtilities.invokeLater(this::refreshAll);
        });
    }

    void runUpdate(ContentCards.Row row, ModrinthVersion version) {
        row.update.setEnabled(false);
        worker.execute(() -> {
            try {
                service.update(row.mod, version, progressOn(row.update, ""));
            } catch (IOException | RuntimeException e) {
                LogManager.warn("Couldn't update " + ContentService.nameOf(row.mod) + ": " + e.getMessage());
                SwingUtilities.invokeLater(() -> {
                    if (App.TOASTER != null) {
                        App.TOASTER.popError(GetText.tr("Couldn't update {0}", ContentService.nameOf(row.mod)));
                    }
                });
            }
            ContentKind kind = ContentKind.of(row.mod.type);
            SwingUtilities.invokeLater(() -> {
                refreshAll();
                if (kind != null) {
                    checkUpdates(kind);
                }
            });
        });
    }

    void runUpdates(ContentPanel panel, java.util.IdentityHashMap<DisableableMod, ModrinthVersion> all) {
        worker.execute(() -> {
            int failed = 0;
            for (Map.Entry<DisableableMod, ModrinthVersion> entry : all.entrySet()) {
                try {
                    service.update(entry.getKey(), entry.getValue(), ContentService.NO_PROGRESS);
                } catch (IOException | RuntimeException e) {
                    failed++;
                }
            }
            int failures = failed;
            SwingUtilities.invokeLater(() -> {
                refreshAll();
                checkUpdates(panel.kind);
                if (failures > 0 && App.TOASTER != null) {
                    App.TOASTER.popError(GetText.tr("{0} couldn't be updated", failures));
                }
            });
        });
    }

    // ---------------------------------------------------------------- files from the computer

    void chooseFiles(ContentKind kind) {
        JFileChooser chooser = new JFileChooser();
        chooser.setMultiSelectionEnabled(true);
        chooser.setDialogTitle(GetText.tr("Add {0}", title(kind).toLowerCase(java.util.Locale.ROOT)));
        chooser.setFileFilter(new FileNameExtensionFilter(GetText.tr("Mods and packs (.jar, .zip)"), "jar", "zip"));
        if (chooser.showOpenDialog(this) == JFileChooser.APPROVE_OPTION) {
            addFiles(java.util.Arrays.asList(chooser.getSelectedFiles()));
        }
    }

    /** Adds files the player dropped or picked: each is looked at and goes where it belongs. */
    void addFiles(List<File> files) {
        FileDropFlow.handle(this, files, instance, i -> {
            refreshAll();
            checkUpdates();
        });
    }

    /** The drop handling of this window, for showing its overlay. */
    WindowFileDrop fileDrop() {
        return fileDrop;
    }
}
