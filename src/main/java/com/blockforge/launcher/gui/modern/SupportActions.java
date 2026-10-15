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

import java.nio.file.Path;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.List;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.diagnostics.CrashClassifier;
import com.blockforge.launcher.diagnostics.Diagnosis;
import com.blockforge.launcher.diagnostics.SupportBundle;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.OS;

/**
 * Support bundle and crash help, shared by the Tools screen, the instance menu and the crash popup.
 */
public final class SupportActions {
    private SupportActions() {
    }

    /**
     * Creates a support bundle in the downloads folder and opens it. Pass null for just the launcher's own log.
     */
    public static void createBundle(Instance instance) {
        SupportBundle.Input input = new SupportBundle.Input();
        input.launcherLog = FileSystem.LOGS.resolve("blockforge.log");
        input.facts.add("Launcher: " + Constants.LAUNCHER_NAME + " " + Constants.VERSION.toStringForLogging());
        input.facts.add("System: " + System.getProperty("os.name") + " " + System.getProperty("os.version") + " ("
            + System.getProperty("os.arch") + ")");
        input.facts.add("Launcher Java: " + System.getProperty("java.version") + " (" + System.getProperty("java.vendor")
            + ", " + System.getProperty("os.arch") + ")");

        for (MicrosoftAccount account : AccountManager.getAccounts()) {
            input.secrets.add(account.minecraftUsername);
            input.secrets.add(account.username);
            input.secrets.add(account.uuid);
            input.secrets.add(account.accessToken);
        }

        if (instance != null) {
            input.instanceRoot = instance.getRoot();
            input.instanceName = instance.getName();
            input.facts.add("Minecraft: " + instance.id);
            if (instance.launcher.loaderVersion != null) {
                input.facts.add("Loader: " + instance.launcher.loaderVersion.type + " "
                    + instance.launcher.loaderVersion.version);
            }
            input.facts.add("Java used by the instance: " + instance.getJavaPath());
            input.facts.add("Memory: " + java.util.Optional.ofNullable(instance.launcher.maximumMemory)
                .orElse(App.settings.maximumMemory) + " MB");
        }

        Path zip = FileSystem.getUserDownloadsPath(true)
            .resolve("BlockForge-support-" + new SimpleDateFormat("yyyyMMdd-HHmmss").format(new Date()) + ".zip");

        try {
            List<Diagnosis> found = SupportBundle.create(zip, input);
            LogManager.info("Created support bundle " + zip + (found.isEmpty() ? "" : " (" + found.get(0) + ")"));
            App.TOASTER.pop(GetText.tr("Support bundle saved to your downloads"));
            OS.openFileExplorer(zip, true);
        } catch (Exception e) {
            LogManager.logStackTrace("Couldn't create the support bundle", e);
            DialogManager.okDialog().setTitle(GetText.tr("Support bundle"))
                .setContent(GetText.tr("The support bundle couldn't be created. See the log for details."))
                .setType(DialogManager.ERROR).show();
        }
    }

    /** The instance that was played last, which is where a person most likely wants help with. */
    public static Instance lastPlayedInstance() {
        return InstanceManager.getInstances().stream()
            .max(java.util.Comparator.comparing(Instance::getLastPlayedOrEpoch)).orElse(null);
    }

    /**
     * After a crash: reads the game log, and if the cause is recognised, explains it and offers a support bundle.
     */
    public static void showCrashHelp(Instance instance) {
        new Thread(() -> {
            try {
                Path latest = instance.getRoot().resolve("logs/latest.log");
                String log = java.nio.file.Files.isRegularFile(latest)
                    ? new String(java.nio.file.Files.readAllBytes(latest), java.nio.charset.StandardCharsets.UTF_8)
                    : "";
                List<Diagnosis> found = CrashClassifier.classify(log);
                if (found.isEmpty()) {
                    return;
                }

                Diagnosis d = found.get(0);
                LogManager.error("Likely cause [" + d.code + "]: " + d.title + ". " + d.fix);

                javax.swing.SwingUtilities.invokeLater(() -> {
                    int ret = DialogManager.yesNoDialog().setTitle(d.title)
                        .setContent("<html><body style='width:380px'><b>" + escape(d.explanation) + "</b><br/><br/>"
                            + escape(d.fix) + "<br/><br/><i>" + GetText.tr("Code") + ": " + d.code
                            + "</i><br/><br/>" + GetText.tr("Create a support bundle you can share?")
                            + "</body></html>")
                        .setType(DialogManager.ERROR).show();
                    if (ret == DialogManager.YES_OPTION) {
                        createBundle(instance);
                    }
                });
            } catch (Exception e) {
                LogManager.debug("Couldn't work out why the game crashed: " + e.getMessage());
            }
        }, "crash-diagnosis").start();
    }

    private static String escape(String text) {
        return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
    }
}
