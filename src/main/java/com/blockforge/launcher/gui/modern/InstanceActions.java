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

import java.awt.Component;
import java.util.ArrayList;
import java.util.List;

import javax.swing.JMenu;
import javax.swing.JMenuItem;
import javax.swing.JPopupMenu;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.data.BackupMode;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.gui.dialogs.AddModsDialog;
import com.blockforge.launcher.gui.dialogs.EditModsDialog;
import com.blockforge.launcher.gui.dialogs.InstanceExportDialog;
import com.blockforge.launcher.gui.dialogs.InstanceSettingsDialog;
import com.blockforge.launcher.gui.dialogs.ProgressDialog;
import com.blockforge.launcher.managers.ConfigManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.snapshots.InstanceSnapshots;
import com.blockforge.launcher.snapshots.SnapshotService;
import com.blockforge.launcher.utils.OS;

/**
 * Everything you can do to an instance from the interface: playing it, updating it, editing it and so on.
 */
public final class InstanceActions {
    private InstanceActions() {
    }

    /**
     * Launches the instance, after the checks that make sure it can actually be launched.
     */
    public static void play(Instance instance, boolean offline, boolean hasUpdate) {
        if (App.launcher.minecraftLaunched) {
            App.TOASTER.pop(GetText.tr("Minecraft is already running"));
            return;
        }

        if (!instance.launcher.isPlayable) {
            DialogManager.okDialog().setTitle(GetText.tr("Instance Corrupt"))
                .setContent(GetText.tr("Cannot play instance as it's corrupted. Please reinstall, update or delete it."))
                .setType(DialogManager.ERROR).show();
            return;
        }

        if (!App.settings.ignoreJavaOnInstanceLaunch && instance.shouldShowWrongJavaWarning()) {
            DialogManager.okDialog().setTitle(GetText.tr("Cannot launch instance due to your Java version"))
                .setContent(new HTMLBuilder().center().text(GetText.tr(
                    "There was an issue launching this instance.<br/><br/>This version of the pack requires a Java version which you are not using.<br/><br/>Please install that version of Java and try again.<br/><br/>Java version needed: {0}",
                    instance.launcher.java.getVersionString())).build())
                .setType(DialogManager.ERROR).show();
            return;
        }

        if (hasUpdate && !instance.hasLatestUpdateBeenIgnored()) {
            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Update Available"))
                .setContent(new HTMLBuilder().center()
                    .text(GetText.tr("An update is available for this instance.<br/><br/>Do you want to update now?"))
                    .build())
                .addOption(GetText.tr("Ignore This Update"))
                .addOption(GetText.tr("Don't Remind Me Again")).setType(DialogManager.INFO).show();

            if (ret == 0) {
                instance.update();
                return;
            }

            if (ret == 2) {
                instance.ignoreUpdate();
            } else if (ret == 3) {
                instance.ignoreAllUpdates();
            }
        }

        if (instance.launch(offline)) {
            App.launcher.setMinecraftLaunched(true);
        }
    }

    public static void delete(Instance instance) {
        int ret = DialogManager.yesNoDialog(false).setTitle(GetText.tr("Delete Instance"))
            .setContent(GetText.tr("Are you sure you want to delete the instance \"{0}\"?", instance.launcher.name))
            .setType(DialogManager.ERROR).show();

        if (ret != DialogManager.YES_OPTION) {
            return;
        }

        final ProgressDialog<Object> dialog = new ProgressDialog<>(GetText.tr("Deleting Instance"), 0,
            GetText.tr("Deleting Instance. Please wait..."), null, App.launcher.getParent());
        dialog.addThread(new Thread(() -> {
            InstanceManager.removeInstance(instance);
            dialog.close();
            App.TOASTER.pop(GetText.tr("Deleted Instance Successfully"));
        }));
        dialog.start();
    }

    public static void openSettings(Instance instance) {
        new InstanceSettingsDialog(instance).setVisible(true);
    }

    public static void openFolder(Instance instance) {
        OS.openFileExplorer(instance.getRoot());
    }

    private static JMenuItem item(String label, Runnable action) {
        JMenuItem item = new JMenuItem(label);
        item.addActionListener(e -> action.run());
        return item;
    }

    /**
     * Builds the "more" menu for an instance. {@code changed} runs after something that alters the instance.
     */
    public static JPopupMenu menu(Instance instance, boolean hasUpdate, Runnable changed) {
        JPopupMenu menu = new JPopupMenu();

        menu.add(item(GetText.tr("Settings"), () -> openSettings(instance)));

        boolean modsViaPlatforms = instance.launcher.enableCurseForgeIntegration
            && (ConfigManager.getConfigItem("platforms.curseforge.modsEnabled", true)
                || (ConfigManager.getConfigItem("platforms.modrinth.modsEnabled", true)
                    && instance.launcher.loaderVersion != null));
        if (modsViaPlatforms) {
            menu.add(item(GetText.tr("Add Mods"), () -> {
                new AddModsDialog(instance).setVisible(true);
                changed.run();
            }));
        }
        if (instance.launcher.enableEditingMods) {
            menu.add(item(GetText.tr("Edit Mods"), () -> {
                new EditModsDialog(instance).setVisible(true);
                changed.run();
            }));
        }

        menu.addSeparator();
        if (instance.isUpdatable()) {
            JMenuItem update = item(GetText.tr("Update"), instance::update);
            update.setEnabled(hasUpdate && instance.launcher.isPlayable);
            menu.add(update);
            menu.add(item(GetText.tr("Reinstall"), instance::startReinstall));
        }
        menu.add(item(GetText.tr("Rename"), () -> {
            instance.startRename();
            changed.run();
        }));
        if (instance.canChangeDescription()) {
            menu.add(item(GetText.tr("Change Description"), () -> {
                instance.startChangeDescription();
                changed.run();
            }));
        }
        menu.add(item(GetText.tr("Change Image"), () -> {
            instance.startChangeImage();
            changed.run();
        }));
        menu.add(item(GetText.tr("Clone"), instance::startClone));

        JMenu loader = loaderMenu(instance, changed);
        if (loader.getItemCount() > 0) {
            menu.add(loader);
        }

        menu.add(snapshotsMenu(instance));

        JMenu backup = new JMenu(GetText.tr("Backup"));
        backup.add(item(GetText.tr("Normal Backup"), () -> instance.backup(BackupMode.NORMAL)));
        backup.add(item(GetText.tr("Normal + Mods Backup"), () -> instance.backup(BackupMode.NORMAL_PLUS_MODS)));
        backup.add(item(GetText.tr("Full Backup"), () -> instance.backup(BackupMode.FULL)));
        menu.add(backup);

        menu.addSeparator();
        menu.add(item(GetText.tr("Play Offline With Custom Name"), () -> play(instance, true, false)));
        menu.add(item(GetText.tr("Open Folder"), () -> openFolder(instance)));
        menu.add(item(GetText.tr("Create Support Bundle"), () -> SupportActions.createBundle(instance)));
        if (instance.canBeExported()) {
            menu.add(item(GetText.tr("Export"), () -> new InstanceExportDialog(instance).setVisible(true)));
        }
        if (instance.hasWebsite()) {
            menu.add(item(GetText.tr("Open Website"), () -> OS.openWebBrowser(instance.getWebsiteUrl())));
        }

        menu.addSeparator();
        JMenuItem delete = item(GetText.tr("Delete"), () -> delete(instance));
        delete.setForeground(Palette.danger());
        menu.add(delete);
        return menu;
    }

    /** Snapshots of the pack's files, taken before updates, that can be put back. */
    private static JMenu snapshotsMenu(Instance instance) {
        JMenu menu = new JMenu(GetText.tr("Snapshots"));
        menu.add(item(GetText.tr("Save Snapshot Now"), () -> saveSnapshot(instance)));

        List<InstanceSnapshots.Snapshot> recent = InstanceSnapshots.latest(SnapshotService.folderFor(instance), 6);
        if (!recent.isEmpty()) {
            menu.addSeparator();
        }

        java.text.DateFormat format = java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM,
            java.text.DateFormat.SHORT);
        for (InstanceSnapshots.Snapshot snapshot : recent) {
            String text = GetText.tr("Restore") + ": " + snapshot.label() + " ("
                + format.format(new java.util.Date(snapshot.createdAt)) + ")";
            menu.add(item(text, () -> restoreSnapshot(instance, snapshot)));
        }
        return menu;
    }

    /** Zipping the mods and configs can take a while for a big pack, so it happens behind a progress dialog. */
    private static void saveSnapshot(Instance instance) {
        final ProgressDialog<Boolean> dialog = new ProgressDialog<>(GetText.tr("Saving Snapshot"), 0,
            GetText.tr("Saving Snapshot. Please wait..."), null, App.launcher.getParent());
        dialog.addThread(new Thread(() -> {
            dialog.setReturnValue(SnapshotService.saveBefore(instance, "manual"));
            dialog.close();
        }));
        dialog.start();

        if (Boolean.TRUE.equals(dialog.getReturnValue())) {
            App.TOASTER.pop(GetText.tr("Snapshot saved"));
        } else {
            App.TOASTER.popError(GetText.tr("Couldn't save the snapshot"));
        }
    }

    private static void restoreSnapshot(Instance instance, InstanceSnapshots.Snapshot snapshot) {
        int ret = DialogManager.yesNoDialog(false).setTitle(GetText.tr("Restore Snapshot"))
            .setContent(GetText.tr("Put the mods and configs of \"{0}\" back to how they were? Your worlds are not changed, and the current state is saved first so you can undo this.",
                instance.launcher.name))
            .setType(DialogManager.WARNING).show();
        if (ret != DialogManager.YES_OPTION) {
            return;
        }

        final ProgressDialog<Boolean> dialog = new ProgressDialog<>(GetText.tr("Restoring Snapshot"), 0,
            GetText.tr("Restoring Snapshot. Please wait..."), null, App.launcher.getParent());
        dialog.addThread(new Thread(() -> {
            try {
                InstanceSnapshots.restore(snapshot, instance.getRoot(), SnapshotService.folderFor(instance));
                dialog.setReturnValue(true);
            } catch (Exception e) {
                LogManager.logStackTrace("Couldn't restore the snapshot", e);
                dialog.setReturnValue(false);
            }
            dialog.close();
        }));
        dialog.start();

        if (Boolean.TRUE.equals(dialog.getReturnValue())) {
            App.TOASTER.pop(GetText.tr("Snapshot restored"));
        } else {
            App.TOASTER.popError(GetText.tr("The snapshot couldn't be restored, nothing was changed"));
        }
    }

    private static JMenu loaderMenu(Instance instance, Runnable changed) {
        JMenu loader = new JMenu(GetText.tr("Mod Loader"));

        if (instance.launcher.loaderVersion == null) {
            List<LoaderType> addable = new ArrayList<>();
            if (enabled("fabric", true, instance)) {
                addable.add(LoaderType.FABRIC);
            }
            if (enabled("forge", true, instance)) {
                addable.add(LoaderType.FORGE);
            }
            if (enabled("neoforge", true, instance)) {
                addable.add(LoaderType.NEOFORGE);
            }
            if (enabled("quilt", false, instance)) {
                addable.add(LoaderType.QUILT);
            }
            if (enabled("legacyfabric", true, instance)) {
                addable.add(LoaderType.LEGACY_FABRIC);
            }

            for (LoaderType type : addable) {
                loader.add(item(GetText.tr("Add {0}", type.toString()), () -> {
                    instance.addLoader(type);
                    changed.run();
                }));
            }
        } else {
            loader.add(item(GetText.tr("Change {0} Version", instance.launcher.loaderVersion.type), () -> {
                instance.changeLoaderVersion();
                changed.run();
            }));
            loader.add(item(GetText.tr("Remove {0}", instance.launcher.loaderVersion.type), () -> {
                instance.removeLoader();
                changed.run();
            }));
        }

        return loader;
    }

    private static boolean enabled(String key, boolean defaultValue, Instance instance) {
        return ConfigManager.getConfigItem("loaders." + key + ".enabled", defaultValue)
            && !ConfigManager.getConfigItem("loaders." + key + ".disabledMinecraftVersions", new ArrayList<String>())
                .contains(instance.id);
    }

    /** Shows the instance menu under a component. */
    public static void showMenu(Component invoker, int x, int y, Instance instance, boolean hasUpdate,
        Runnable changed) {
        menu(instance, hasUpdate, changed).show(invoker, x, y);
    }
}
