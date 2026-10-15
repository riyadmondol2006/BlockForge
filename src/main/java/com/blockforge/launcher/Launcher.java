/*
 * ATLauncher - https://github.com/ATLauncher/ATLauncher
 * Copyright (C) 2013-2022 ATLauncher
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
package com.blockforge.launcher;

import java.awt.Dialog.ModalityType;
import java.awt.FlowLayout;
import java.awt.Window;
import java.io.File;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Date;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.stream.Collectors;

import javax.swing.JDialog;
import javax.swing.JFrame;
import javax.swing.JLabel;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.DownloadableFile;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.graphql.AddLauncherLaunchMutation;
import com.blockforge.launcher.graphql.type.AddLauncherLaunchInput;
import com.blockforge.launcher.graphql.type.LauncherJavaVersionInput;
import com.blockforge.launcher.gui.dialogs.ProgressDialog;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.ConfigManager;
import com.blockforge.launcher.managers.CurseForgeUpdateManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.FTBUpdateManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LWJGLManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.managers.MinecraftManager;
import com.blockforge.launcher.managers.ModrinthModpackUpdateManager;
import com.blockforge.launcher.managers.NewsManager;
import com.blockforge.launcher.managers.PackManager;
import com.blockforge.launcher.managers.PerformanceManager;
import com.blockforge.launcher.managers.ServerManager;
import com.blockforge.launcher.managers.TechnicModpackUpdateManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.network.DownloadPool;
import com.blockforge.launcher.network.GraphqlClient;
import com.blockforge.launcher.network.NetworkClient;
import com.blockforge.launcher.utils.Java;
import com.blockforge.launcher.utils.OS;
import com.google.gson.reflect.TypeToken;

import okhttp3.OkHttpClient;

public class Launcher {
    // Holding update data
    private List<DownloadableFile> launcherFiles; // Files the Launcher needs to download

    // UI things
    private JFrame parent; // Parent JFrame of the actual Launcher

    // Update thread
    private Thread updateThread;

    // Minecraft tracking variables
    private Process minecraftProcess = null; // The process minecraft is running on
    public boolean minecraftLaunched = false; // If Minecraft has been Launched
    public Instance lastInstanceCrash = null; // The last instance that crashed
    public Date lastInstanceCrashTime = null; // The time the last instance crashed

    public void loadEverything() {
        PerformanceManager.start();
        if (hasUpdatedFiles()) {
            downloadUpdatedFiles(); // Downloads updated files on the server
        }

        ConfigManager.loadConfig(); // Load the config

        NewsManager.loadNews(); // Load the news

        if (App.settings.enableAnalytics && ConfigManager.getConfigItem("useGraphqlLauncherLaunch", false)) {
            App.TASKPOOL.execute(() -> GraphqlClient.mutate(new AddLauncherLaunchMutation(
                AddLauncherLaunchInput.builder().version(Constants.VERSION.toStringForLogging())
                    .hash(Constants.VERSION.getSha1Revision().toString())
                    .installMethod(OS.getInstallMethod())
                    .javaVersion(LauncherJavaVersionInput.builder().raw(Java.getLauncherJavaVersion())
                        .majorVersion(Integer.toString(Java.getLauncherJavaVersionNumber()))
                        .bitness(Java.is64Bit() ? 64 : 32)
                        .usingJreDir(OS.isWindows() && OS.usingExe()
                            && Files.exists(FileSystem.BASE_DIR.resolve("jre")))
                        .build())
                    .build())));
        }

        // accounts don't depend on anything else, so load them while the Minecraft data and packs load
        CompletableFuture<Void> accounts = CompletableFuture.runAsync(AccountManager::loadAccounts);

        MinecraftManager.loadMinecraftVersions(); // Load info about the different Minecraft versions
        MinecraftManager.loadJavaRuntimes(); // Load info about the different java runtimes
        LWJGLManager.loadLWJGLVersions(); // Load info about the different LWJGL versions

        PackManager.loadPacks(); // Load the Packs available in the Launcher (needs the Minecraft versions)

        PackManager.loadUsers(); // Load the Testers and Allowed Players for the packs

        // instances need the accounts to be loaded first
        accounts.join();

        InstanceManager.loadInstances(); // Load the users installed Instances

        ServerManager.loadServers(); // Load the users installed servers

        PackManager.removeUnusedImages(); // remove unused pack images

        if (OS.isWindows() && !Java.is64Bit() && OS.is64Bit()) {
            LogManager.warn("You're using 32 bit Java on a 64 bit Windows install!");

            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Running 32 Bit Java on 64 Bit Windows"))
                .setContent(new HTMLBuilder().center().text(GetText.tr(
                        "We have detected that you're running 64 bit Windows but not 64 bit Java.<br/><br/>This will cause severe issues playing all packs if not fixed.<br/><br/>Do you want to close the launcher and learn how to fix this issue now?"))
                    .build())
                .setType(DialogManager.ERROR).show();

            if (ret == 0) {
                OS.openWebBrowser("https://atlauncher.com/help/32bit/");
                System.exit(0);
            }
        }

        checkForExternalPackUpdates();

        if (App.settings.enableAnalytics && Analytics.isEnabled()) {
            Analytics.startSession(App.settings.selectedTabOnStartup);
        }
        PerformanceManager.end();
    }

    public void runUpdate(String currentPath, String temporaryUpdatePath) {
        List<String> arguments = new ArrayList<>();

        String path = System.getProperty("java.home") + File.separator + "bin" + File.separator + "java";
        if (OS.isWindows()) {
            path += "w";
        }
        arguments.add(path);
        arguments.add("-cp");
        arguments.add(temporaryUpdatePath);
        arguments.add("com.blockforge.launcher.Update");
        arguments.add(currentPath);
        arguments.add(temporaryUpdatePath);

        // pass in all the original arguments
        arguments.addAll(Arrays.asList(App.PASSED_ARGS));

        ProcessBuilder processBuilder = new ProcessBuilder();
        processBuilder.command(arguments);

        LogManager.info("Running launcher update with command " + arguments);

        try {
            processBuilder.start();
        } catch (IOException e) {
            LogManager.logStackTrace(e);
        }

        Analytics.endSession();
        System.exit(0);
    }

    /**
     * This checks the servers files.json file and gets the files that the Launcher needs to have
     */
    private List<com.blockforge.launcher.network.Download> getLauncherFiles() {
        if (this.launcherFiles == null) {
            // use the list from the last run so start up doesn't wait on the network, then refresh it in the
            // background so any changes are picked up for next time
            List<DownloadableFile> cached = readCachedLauncherFiles();
            if (cached != null) {
                this.launcherFiles = cached;
                CompletableFuture.runAsync(this::refreshLauncherFilesInBackground);
            } else {
                this.launcherFiles = fetchLauncherFiles();
            }
        }

        if (this.launcherFiles == null) {
            return null;
        }

        return this.launcherFiles.stream()
            .filter(file -> !file.isLauncher() && !file.isFiles() && file.isForArchAndOs())
            .map(DownloadableFile::getDownload).collect(Collectors.toList());
    }

    private static final java.lang.reflect.Type LAUNCHER_FILES_TYPE = new TypeToken<List<DownloadableFile>>() {
    }.getType();

    private static Path launcherFilesCache() {
        return FileSystem.JSON.resolve("launcher-files-cache.json");
    }

    private List<DownloadableFile> readCachedLauncherFiles() {
        Path cache = launcherFilesCache();
        if (!Files.exists(cache)) {
            return null;
        }

        try (InputStreamReader reader = new InputStreamReader(Files.newInputStream(cache), StandardCharsets.UTF_8)) {
            return Gsons.DEFAULT.fromJson(reader, LAUNCHER_FILES_TYPE);
        } catch (Exception e) {
            LogManager.debug("Couldn't read cached launcher files list: " + e.getMessage());
            return null;
        }
    }

    private List<DownloadableFile> fetchLauncherFiles() {
        try {
            List<DownloadableFile> files = NetworkClient.get(
                String.format("%s/launcher/json/files.json", Constants.DOWNLOAD_SERVER), LAUNCHER_FILES_TYPE);

            if (files != null) {
                try {
                    Files.write(launcherFilesCache(), Gsons.DEFAULT.toJson(files).getBytes(StandardCharsets.UTF_8));
                } catch (IOException e) {
                    LogManager.debug("Couldn't cache launcher files list: " + e.getMessage());
                }
            }

            return files;
        } catch (Exception e) {
            LogManager.logStackTrace("Error loading in file hashes!", e);
            return null;
        }
    }

    /**
     * Fetches the current list of launcher files and downloads anything that changed, without holding anything up.
     */
    private void refreshLauncherFilesInBackground() {
        List<DownloadableFile> fresh = fetchLauncherFiles();
        if (fresh == null) {
            return;
        }

        this.launcherFiles = fresh;

        List<com.blockforge.launcher.network.Download> downloads = getLauncherFiles();
        if (downloads == null) {
            return;
        }

        DownloadPool pool = new DownloadPool();
        pool.addAll(downloads);
        DownloadPool needed = pool.downsize();
        if (!needed.isEmpty()) {
            LogManager.info("Downloading updated launcher files in the background");
            needed.downloadAll();
        }
    }

    public void downloadUpdatedFiles() {
        ProgressDialog<Object> progressDialog = new ProgressDialog<>(GetText.tr("Downloading Updates"), 1,
            GetText.tr("Downloading Updates"));
        progressDialog.addThread(new Thread(() -> {
            DownloadPool pool = new DownloadPool();
            OkHttpClient httpClient = Network.createProgressClient(progressDialog);
            pool.addAll(
                getLauncherFiles().stream().map(dl -> dl.withHttpClient(httpClient)).collect(Collectors.toList()));
            DownloadPool smallPool = pool.downsize();

            progressDialog.setTotalBytes(smallPool.totalSize());

            pool.downloadAll();
            progressDialog.doneTask();
            progressDialog.close();
        }));
        progressDialog.start();

        LogManager.info("Finished downloading updated files!");
    }

    public boolean checkForUpdatedFiles() {
        this.launcherFiles = fetchLauncherFiles();

        App.TASKPOOL.execute(this::checkForExternalPackUpdates);

        return hasUpdatedFiles();
    }

    /**
     * This checks the servers hashes.json file and looks for new/updated files that differ from what the user has
     */
    public boolean hasUpdatedFiles() {
        LogManager.info("Checking for updated files!");
        List<com.blockforge.launcher.network.Download> downloads = getLauncherFiles();

        if (downloads == null) {
            return false;
        }

        return downloads.stream().anyMatch(com.blockforge.launcher.network.Download::needToDownload);
    }

    public void checkForExternalPackUpdates() {
        if (updateThread != null && updateThread.isAlive()) {
            updateThread.interrupt();
        }

        updateThread = new Thread(() -> {
            if (InstanceManager.getInstances().stream().anyMatch(Instance::isFTBPack)) {
                FTBUpdateManager.checkForUpdates();
            }
            if (InstanceManager.getInstances().stream().anyMatch(Instance::isCurseForgePack)) {
                CurseForgeUpdateManager.checkForUpdates();
            }
            if (InstanceManager.getInstances().stream().anyMatch(Instance::isTechnicPack)) {
                TechnicModpackUpdateManager.checkForUpdates();
            }
            if (InstanceManager.getInstances().stream().anyMatch(Instance::isModrinthPack)) {
                ModrinthModpackUpdateManager.checkForUpdates();
            }
        });
        updateThread.start();
    }

    public void updateData() {
        updateData(false);
    }

    public void updateData(boolean force) {
        if (checkForUpdatedFiles()) {
            reloadLauncherData();
        }

        MinecraftManager.loadMinecraftVersions(); // Load info about the different Minecraft versions
        MinecraftManager.loadJavaRuntimes(); // Load info about the different java runtimes
    }

    public void reloadLauncherData() {
        final JDialog dialog = new JDialog(this.parent, ModalityType.DOCUMENT_MODAL);
        dialog.setSize(300, 100);
        dialog.setTitle("Updating Launcher");
        dialog.setLocationRelativeTo(App.launcher.getParent());
        dialog.setLayout(new FlowLayout());
        dialog.setResizable(false);
        dialog.add(new JLabel(GetText.tr("Updating Launcher. Please Wait")));
        App.TASKPOOL.execute(() -> {
            if (hasUpdatedFiles()) {
                downloadUpdatedFiles(); // Downloads updated files on the server
            }
            checkForExternalPackUpdates();

            ConfigManager.loadConfig(); // Load the config
            NewsManager.loadNews(); // Load the news
            PackManager.loadPacks(); // Load the Packs available in the Launcher
            reloadPacksBrowserPanel();// Reload packs browser panel
            PackManager.loadUsers(); // Load the Testers and Allowed Players for the packs
            InstanceManager.loadInstances(); // Load the users installed Instances
            dialog.setVisible(false); // Remove the dialog
            dialog.dispose(); // Dispose the dialog
        });
        dialog.setVisible(true);
    }

    /**
     * Sets the main parent JFrame reference for the Launcher
     *
     * @param parent The Launcher main JFrame
     */
    public void setParentFrame(JFrame parent) {
        this.parent = parent;
    }

    public void setMinecraftLaunched(boolean launched) {
        this.minecraftLaunched = launched;
        if (App.TRAY_MENU != null) {
            App.TRAY_MENU.setMinecraftLaunched(launched);
        }
    }

    /**
     * Returns the JFrame reference of the main Launcher
     *
     * @return Main JFrame of the Launcher
     */
    public Window getParent() {
        return this.parent;
    }

    /**
     * Reloads the Discover screen, if it has been opened
     */
    public void reloadPacksBrowserPanel() {
        if (App.launcherFrame != null) {
            App.launcherFrame.refreshPacks();
        }
    }

    /**
     * Called when the account or the added pack codes change. The old packs tab filtered its list by those; the
     * Discover screen's results come from the modpack search and don't depend on them, so there is nothing to do.
     * (Searching again here made every account switch throw away the results on screen and hit the network.)
     */
    public void refreshPacksBrowserPanel() {
    }

    public void showKillMinecraft(Process minecraft) {
        this.minecraftProcess = minecraft;
        this.lastInstanceCrash = null;
        this.lastInstanceCrashTime = null;
        App.console.showKillMinecraft();

        if (App.launcherFrame != null) {
            App.launcherFrame.setMinecraftRunning(true);
        }
    }

    public void hideKillMinecraft() {
        App.console.hideKillMinecraft();

        if (App.launcherFrame != null) {
            App.launcherFrame.setMinecraftRunning(false);
        }
    }

    public void killMinecraft() {
        if (this.minecraftProcess != null) {
            LogManager.error("Killing Minecraft");

            this.minecraftProcess.destroy();
            this.minecraftProcess = null;
        } else {
            LogManager.error("Cannot kill Minecraft as there is no instance open!");
        }
    }

    public void setLastInstanceCrash(Instance instance) {
        this.lastInstanceCrash = instance;
        this.lastInstanceCrashTime = new Date();
    }
}
