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


import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.content.ContentKind;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
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
        play(instance, hasUpdate, () -> instance.launch(offline));
    }

    /** Launches the instance straight into one of its singleplayer worlds, just this once. */
    public static void continueWorld(Instance instance, String worldFolder) {
        play(instance, false, () -> instance.launchIntoWorld(worldFolder));
    }

    private static void play(Instance instance, boolean hasUpdate,
        java.util.function.BooleanSupplier launch) {
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

        if (launch.getAsBoolean()) {
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

    /** Opens the mods, resource packs and shaders of an instance on the given tab. */
    public static void openContent(Instance instance, ContentKind tab, Runnable changed) {
        ContentManagerDialog.open(App.launcherFrame, instance, tab, changed);
    }

    public static void openFolder(Instance instance) {
        OS.openFileExplorer(instance.getRoot());
    }

    /** Whether the classic "Add mods" window has anything to offer for this instance. */
    static boolean modsViaPlatforms(Instance instance) {
        return instance.launcher.enableCurseForgeIntegration
            && (ConfigManager.getConfigItem("platforms.curseforge.modsEnabled", true)
                || (ConfigManager.getConfigItem("platforms.modrinth.modsEnabled", true)
                    && instance.launcher.loaderVersion != null));
    }

    /** The mod loaders that can be added to an instance that doesn't have one. */
    static List<LoaderType> addableLoaders(Instance instance) {
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
        return addable;
    }

    /** Zipping the mods and configs can take a while for a big pack, so it happens behind a progress dialog. */
    static void saveSnapshot(Instance instance) {
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

    static void restoreSnapshot(Instance instance, InstanceSnapshots.Snapshot snapshot) {
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

    private static boolean enabled(String key, boolean defaultValue, Instance instance) {
        return ConfigManager.getConfigItem("loaders." + key + ".enabled", defaultValue)
            && !ConfigManager.getConfigItem("loaders." + key + ".disabledMinecraftVersions", new ArrayList<String>())
                .contains(instance.id);
    }

    /** Shows the instance menu at a point, like where the user right clicked. */
    public static void showMenu(Component invoker, int x, int y, Instance instance, boolean hasUpdate,
        Runnable changed) {
        InstanceMenu.showAt(invoker, x, y, instance, hasUpdate, changed);
    }

    /** Drops the instance menu down from its "more" button, or opens it above near the bottom of the window. */
    public static void showMenuBelow(Component button, Instance instance, boolean hasUpdate, Runnable changed) {
        InstanceMenu.showBelow(button, instance, hasUpdate, changed);
    }
}
