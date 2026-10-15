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
import java.awt.Cursor;
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Frame;
import java.awt.GridBagLayout;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JDialog;
import javax.swing.JFileChooser;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;
import javax.swing.WindowConstants;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.installables.Installable;
import com.blockforge.launcher.data.installables.VanillaInstallable;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.gui.dialogs.ImportInstanceDialog;
import com.blockforge.launcher.gui.dialogs.ProgressDialog;
import com.blockforge.launcher.imports.ImportedInstance;
import com.blockforge.launcher.imports.InstanceImporter;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.managers.MinecraftManager;
import com.blockforge.launcher.utils.Utils;

/**
 * Bring instances over from Prism, MultiMC, CurseForge, GDLauncher or ATLauncher. Launchers installed on this computer
 * are found automatically, and a folder can be picked by hand.
 */
public final class ImportWizardDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private static final ExecutorService PREVIEWS = Executors.newSingleThreadExecutor(r -> {
        Thread thread = new Thread(r, "import-preview");
        thread.setDaemon(true);
        return thread;
    });

    private final JPanel list = new JPanel();
    private final JLabel status = Lbl.dim(GetText.tr("Looking for other launchers…"));

    private ImportWizardDialog(Frame owner) {
        super(owner, GetText.tr("Import instances"), Dialog.ModalityType.APPLICATION_MODAL);
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
        setIconImage(Utils.getImage("/assets/image/icon.png"));

        JPanel root = new JPanel(new BorderLayout()) {
            private static final long serialVersionUID = 1L;

            @Override
            public java.awt.Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(26, 30, 24, 30));

        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        JLabel title = Lbl.title(GetText.tr("Import instances"));
        JLabel sub = Lbl.dim(GetText.tr("Your files are copied, nothing is changed or removed in the other launcher."));
        title.setAlignmentX(Component.LEFT_ALIGNMENT);
        sub.setAlignmentX(Component.LEFT_ALIGNMENT);
        status.setAlignmentX(Component.LEFT_ALIGNMENT);
        header.add(title);
        header.add(sub);
        header.add(Box.createVerticalStrut(14));
        header.add(status);
        header.add(Box.createVerticalStrut(8));
        root.add(header, BorderLayout.NORTH);

        list.setOpaque(false);
        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        JPanel wrap = new JPanel(new BorderLayout());
        wrap.setOpaque(false);
        wrap.add(list, BorderLayout.NORTH);
        root.add(FitWidthPanel.scroll(wrap), BorderLayout.CENTER);

        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.LEFT, 10, 0));
        buttons.setOpaque(false);
        buttons.setBorder(BorderFactory.createEmptyBorder(14, 0, 0, 0));
        Btn folder = new Btn(GetText.tr("Choose a folder"), Btn.Kind.TONAL).icon(Icons.FOLDER);
        folder.addActionListener(e -> chooseFolder());
        Btn file = new Btn(GetText.tr("Import a file"), Btn.Kind.TONAL).icon(Icons.IMPORT);
        file.addActionListener(e -> {
            dispose();
            new ImportInstanceDialog().setVisible(true);
        });
        buttons.add(folder);
        buttons.add(file);
        root.add(buttons, BorderLayout.SOUTH);

        add(root);
        setPreferredSize(new Dimension(620, 560));
        pack();
        setLocationRelativeTo(owner);

        new Thread(() -> {
            List<ImportedInstance> found = InstanceImporter.scan(InstanceImporter.defaultRoots());
            SwingUtilities.invokeLater(() -> showFound(found));
        }, "import-scan").start();
    }

    public static void open(Frame owner) {
        new ImportWizardDialog(owner).setVisible(true);
    }

    private void showFound(List<ImportedInstance> found) {
        list.removeAll();
        if (found.isEmpty()) {
            status.setText(GetText.tr("No other launchers were found. Choose an instance folder yourself."));
        } else {
            status.setText(found.size() == 1 ? GetText.tr("Found 1 instance") : GetText.tr("Found {0} instances", found.size()));
        }
        for (ImportedInstance instance : found) {
            list.add(row(instance));
            list.add(Box.createVerticalStrut(8));
        }
        list.revalidate();
        list.repaint();
    }

    private void chooseFolder() {
        JFileChooser chooser = new JFileChooser();
        chooser.setFileSelectionMode(JFileChooser.DIRECTORIES_ONLY);
        chooser.setDialogTitle(GetText.tr("Choose an instance folder"));
        if (chooser.showOpenDialog(this) != JFileChooser.APPROVE_OPTION) {
            return;
        }

        File chosen = chooser.getSelectedFile();
        try {
            ImportedInstance instance = InstanceImporter.read(chosen.toPath());
            List<ImportedInstance> single = new ArrayList<>();
            single.add(instance);
            showFound(single);
        } catch (IOException e) {
            DialogManager.okDialog().setTitle(GetText.tr("Can't import this folder"))
                .setContent(e.getMessage()).setType(DialogManager.ERROR).show();
        }
    }

    private RoundPanel row(ImportedInstance instance) {
        RoundPanel row = new RoundPanel(new BorderLayout(14, 0), 20, Palette::card);
        row.line(Palette::outline);
        row.hover(Palette::cardHover);
        row.pad(12, 16, 12, 12);
        row.setAlignmentX(Component.LEFT_ALIGNMENT);
        row.setMaximumSize(new Dimension(Integer.MAX_VALUE, 82));

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel name = Lbl.text(instance.name, 15f, true, Palette::text);
        JLabel details = Lbl.text(instance.source + "  ·  Minecraft " + instance.minecraftVersion + "  ·  "
            + instance.loaderLabel(), 12f, false, Palette::textDim);
        JLabel counts = Lbl.text(" ", 12f, false, Palette::textDim);
        text.add(name);
        text.add(details);
        text.add(counts);
        row.add(text, BorderLayout.CENTER);

        Btn importButton = new Btn(GetText.tr("Import"), Btn.Kind.FILLED).icon(Icons.DOWNLOAD).compact();
        importButton.addActionListener(e -> importInstance(instance));
        JPanel right = new JPanel(new GridBagLayout());
        right.setOpaque(false);
        right.add(importButton);
        row.add(right, BorderLayout.EAST);

        // what is inside is worked out in the background so the list appears straight away. One at a time: walking
        // dozens of instances' files at once (a thread each) thrashes the disk and makes every row slow
        PREVIEWS.execute(() -> {
            try {
                InstanceImporter.Preview p = InstanceImporter.preview(instance);
                String summary = p.mods + " " + GetText.tr("mods") + "  ·  " + p.worlds + " " + GetText.tr("worlds")
                    + "  ·  " + size(p.bytes);
                SwingUtilities.invokeLater(() -> counts.setText(summary));
            } catch (IOException e) {
                SwingUtilities.invokeLater(() -> counts.setText(GetText.tr("Couldn't read this instance's files")));
            }
        });

        row.setCursor(Cursor.getPredefinedCursor(Cursor.DEFAULT_CURSOR));
        row.addMouseListener(new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                // rows are not clickable, the button does the work
            }
        });
        return row;
    }

    private void importInstance(ImportedInstance imported) {
        try {
            MinecraftManager.getMinecraftVersion(imported.minecraftVersion);
        } catch (Exception e) {
            DialogManager.okDialog().setTitle(GetText.tr("Can't import this instance"))
                .setContent(GetText.tr("Minecraft {0} isn't available to install.", imported.minecraftVersion))
                .setType(DialogManager.ERROR).show();
            return;
        }

        dispose();

        // an instance with the same name (say, one imported before) would stop the install with "rename it and try
        // again", and the import has nowhere to rename it, so a free name is picked instead
        String name = InstanceImporter.uniqueName(imported.name, InstanceManager::isInstance);

        try {
            final Installable installable = new VanillaInstallable(
                MinecraftManager.getMinecraftVersion(imported.minecraftVersion), null, name);
            installable.instanceName = name;
            installable.isReinstall = false;
            installable.isServer = false;

            if (!installable.startInstall()) {
                return;
            }

            Instance created = InstanceManager.getInstanceByName(name);
            if (created == null) {
                LogManager.error("Imported instance " + name + " was created but couldn't be found");
                return;
            }

            copyFiles(imported, created);
        } catch (Exception e) {
            LogManager.logStackTrace("Failed to import " + imported.name, e);
            DialogManager.okDialog().setTitle(GetText.tr("Import failed"))
                .setContent(GetText.tr("The instance couldn't be imported. See the log for details."))
                .setType(DialogManager.ERROR).show();
        }
    }

    private void copyFiles(ImportedInstance imported, Instance created) {
        final ProgressDialog<Boolean> dialog = new ProgressDialog<>(GetText.tr("Importing"), 0,
            GetText.tr("Copying files from {0}", imported.source), null, App.launcher.getParent());
        dialog.addThread(new Thread(() -> {
            try {
                InstanceImporter.copyInto(imported, created.getRoot());
                dialog.setReturnValue(true);
            } catch (IOException e) {
                LogManager.logStackTrace("Failed to copy files for " + imported.name, e);
                dialog.setReturnValue(false);
            }
            dialog.close();
        }));
        dialog.start();

        if (!Boolean.TRUE.equals(dialog.getReturnValue())) {
            DialogManager.okDialog().setTitle(GetText.tr("Import failed"))
                .setContent(GetText.tr("The files couldn't be copied, so nothing was changed in the new instance."))
                .setType(DialogManager.ERROR).show();
            return;
        }

        if (imported.maximumMemoryMb != null) {
            created.launcher.maximumMemory = imported.maximumMemoryMb;
        }
        if (imported.javaArguments != null && !imported.javaArguments.isEmpty()) {
            created.launcher.javaArguments = imported.javaArguments;
        }
        created.save();
        App.TOASTER.pop(GetText.tr("Imported {0}", imported.name));

        LoaderType loader = loaderType(imported.loaderType);
        if (loader != null) {
            DialogManager.okDialog().setTitle(GetText.tr("Add the mod loader"))
                .setContent(GetText.tr("{0} used {1}. Choose the version to add it to the new instance.",
                    imported.name, imported.loaderLabel()))
                .setType(DialogManager.INFO).show();
            created.addLoader(loader);
        }
    }

    private static String size(long bytes) {
        if (bytes < 1024L * 1024) {
            return Math.max(1, bytes / 1024) + " KB";
        }
        if (bytes < 1024L * 1024 * 1024) {
            return String.format("%.0f MB", bytes / (1024.0 * 1024));
        }
        return String.format("%.1f GB", bytes / (1024.0 * 1024 * 1024));
    }

    private static LoaderType loaderType(String type) {
        if (type == null) {
            return null;
        }
        switch (type) {
            case "fabric":
                return LoaderType.FABRIC;
            case "forge":
                return LoaderType.FORGE;
            case "neoforge":
                return LoaderType.NEOFORGE;
            case "quilt":
                return LoaderType.QUILT;
            default:
                return null;
        }
    }
}
