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
package com.blockforge.launcher.gui.dialogs;

import java.awt.BorderLayout;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GridLayout;
import java.awt.Window;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.stream.Collectors;
import java.util.stream.Stream;

import javax.swing.BorderFactory;
import javax.swing.BoxLayout;
import javax.swing.JButton;
import javax.swing.JComboBox;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JScrollPane;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.data.AddModRestriction;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.ModManagement;
import com.blockforge.launcher.data.Server;
import com.blockforge.launcher.data.minecraft.loaders.LoaderVersion;
import com.blockforge.launcher.data.modrinth.ModrinthDependency;
import com.blockforge.launcher.data.modrinth.ModrinthDependencyType;
import com.blockforge.launcher.data.modrinth.ModrinthDownloadMetadata;
import com.blockforge.launcher.data.modrinth.ModrinthFile;
import com.blockforge.launcher.data.modrinth.ModrinthProject;
import com.blockforge.launcher.data.modrinth.ModrinthProjectType;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.exceptions.InvalidMinecraftVersion;
import com.blockforge.launcher.gui.card.ModrinthProjectDependencyCard;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.managers.ConfigManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.managers.MinecraftManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.utils.ComboItem;
import com.blockforge.launcher.utils.ModrinthApi;
import com.blockforge.launcher.utils.OS;

public class ModrinthVersionSelectorDialog extends JDialog {
    private int versionsLength = 0;
    private int filesLength = 0;
    private final ModrinthProject project;
    private final ModManagement instanceOrServer;
    private final com.blockforge.launcher.data.Type installType;
    private ModrinthDownloadMetadata.Reason downloadReason = ModrinthDownloadMetadata.Reason.STANDALONE;
    private String installedVersionId = null;
    private String dependentVersionId = null;
    private boolean selectNewest = true;

    private final JPanel dependenciesPanel = new JPanel(new FlowLayout());
    private JScrollPane scrollPane;
    private JButton addButton;
    private JButton viewModButton;
    private JButton viewFileButton;
    private JLabel versionsLabel;
    private JLabel installedJLabel;
    private JComboBox<ModrinthVersion> versionsDropdown;
    private final List<ModrinthVersion> versions = new ArrayList<>();
    private List<ModrinthVersion> versionsData;

    private JLabel filesLabel;
    private JComboBox<ComboItem<ModrinthFile>> filesDropdown;

    public ModrinthVersionSelectorDialog(ModrinthProject mod, ModManagement instanceOrServer) {
        this(App.launcher.getParent(), mod, instanceOrServer, (com.blockforge.launcher.data.Type) null);
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer) {
        this(parent, mod, instanceOrServer, (com.blockforge.launcher.data.Type) null);
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer,
        com.blockforge.launcher.data.Type installType) {
        super(parent, ModalityType.DOCUMENT_MODAL);

        this.project = mod;
        this.instanceOrServer = instanceOrServer;
        this.installType = installType;

        setupComponents();
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer,
        ModrinthDownloadMetadata.Reason downloadReason) {
        this(parent, mod, instanceOrServer, downloadReason, null);
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer,
        ModrinthDownloadMetadata.Reason downloadReason, String dependentVersionId) {
        this(parent, mod, instanceOrServer, (com.blockforge.launcher.data.Type) null);

        this.downloadReason = downloadReason;
        this.dependentVersionId = dependentVersionId;
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, List<ModrinthVersion> versions,
        ModManagement instanceOrServer, String installedVersionId) {
        this(parent, mod, versions, instanceOrServer, installedVersionId, null);
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, List<ModrinthVersion> versions,
        ModManagement instanceOrServer, String installedVersionId, com.blockforge.launcher.data.Type installType) {
        super(parent, ModalityType.DOCUMENT_MODAL);

        this.project = mod;
        this.versionsData = versions;
        this.instanceOrServer = instanceOrServer;
        this.installedVersionId = installedVersionId;
        this.installType = installType;

        setupComponents();
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, List<ModrinthVersion> versions,
        ModManagement instanceOrServer, String installedVersionId, com.blockforge.launcher.data.Type installType,
        ModrinthDownloadMetadata.Reason downloadReason) {
        this(parent, mod, versions, instanceOrServer, installedVersionId, installType);

        this.downloadReason = downloadReason;
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer,
        String installedVersionId, boolean selectNewest) {
        this(parent, mod, instanceOrServer, installedVersionId, selectNewest, null);
    }

    public ModrinthVersionSelectorDialog(Window parent, ModrinthProject mod, ModManagement instanceOrServer,
        String installedVersionId, boolean selectNewest, com.blockforge.launcher.data.Type installType) {
        super(parent, ModalityType.DOCUMENT_MODAL);

        this.project = mod;
        this.instanceOrServer = instanceOrServer;
        this.installedVersionId = installedVersionId;
        this.selectNewest = selectNewest;
        this.installType = installType;

        setupComponents();
    }

    private com.blockforge.launcher.data.Type getEffectiveInstallType(ModrinthVersion version) {
        if (installType != null) {
            return installType;
        }

        if (project.projectType == ModrinthProjectType.DATAPACK
            || (version != null && version.loaders != null && version.loaders.contains("datapack"))) {
            return com.blockforge.launcher.data.Type.datapack;
        }

        if (project.projectType == ModrinthProjectType.SHADER) {
            return com.blockforge.launcher.data.Type.shaderpack;
        }

        if (project.projectType == ModrinthProjectType.RESOURCEPACK) {
            return com.blockforge.launcher.data.Type.resourcepack;
        }

        if (project.projectType == ModrinthProjectType.PLUGIN) {
            return com.blockforge.launcher.data.Type.plugins;
        }

        return com.blockforge.launcher.data.Type.mods;
    }

    private String getProjectUrlPath() {
        com.blockforge.launcher.data.Type effectiveType =
            getEffectiveInstallType(versionsDropdown.getSelectedItem() instanceof ModrinthVersion
                ? (ModrinthVersion) versionsDropdown.getSelectedItem()
                : null);

        if (effectiveType == com.blockforge.launcher.data.Type.datapack) {
            return "datapack";
        }

        if (project.projectType == ModrinthProjectType.RESOURCEPACK) {
            return "resourcepack";
        }

        if (project.projectType == ModrinthProjectType.SHADER) {
            return "shader";
        }

        if (project.projectType == ModrinthProjectType.PLUGIN) {
            return "plugin";
        }

        return "mod";
    }

    public String getSelectedVersionId() {
        Object selected = versionsDropdown.getSelectedItem();
        return selected instanceof ModrinthVersion ? ((ModrinthVersion) selected).id : null;
    }

    public void reloadDependenciesPanel() {
        if (versionsDropdown.getSelectedItem() == null) {
            return;
        }

        ModrinthVersion selectedFile = (ModrinthVersion) versionsDropdown.getSelectedItem();

        dependenciesPanel.setVisible(false);

        List<ModrinthDependency> dependencies = selectedFile.dependencies.stream().filter(d -> d.projectId != null)
            .collect(Collectors.toList());

        // this file has dependencies
        if (!dependencies.isEmpty()) {
            // check to see which required ones we don't already have
            List<ModrinthDependency> dependenciesNeeded = dependencies.stream()
                .filter(dependency -> dependency.dependencyType == ModrinthDependencyType.REQUIRED)
                .filter(dependency -> {
                    if (!dependency.projectId.equals(Constants.MODRINTH_FABRIC_MOD_ID)) {
                        return true;
                    }

                    // We shouldn't install Fabric API when using Sinytra Connector
                    return instanceOrServer instanceof Server
                        || !((Instance) instanceOrServer).isForgeLikeAndHasInstalledSinytraConnector();
                })
                .filter(dependency -> instanceOrServer.getMods().stream()
                    .noneMatch(installedMod -> {
                        // don't show Modrinth dependency when grabbed from CurseForge
                        if (dependency.projectId.equals(Constants.MODRINTH_FABRIC_MOD_ID)
                            && installedMod.isFromCurseForge()
                            && installedMod
                            .getCurseForgeFileId() == Constants.CURSEFORGE_FABRIC_MOD_ID) {
                            return true;
                        }

                        // don't show Modrinth dependency when grabbed from CurseForge
                        if (dependency.projectId.equals(Constants.MODRINTH_LEGACY_FABRIC_MOD_ID)
                            && installedMod.isFromCurseForge()
                            && installedMod
                            .getCurseForgeFileId() == Constants.CURSEFORGE_LEGACY_FABRIC_MOD_ID) {
                            return true;
                        }

                        // don't show Fabric dependency when QSL is installed
                        if (dependency.projectId.equals(Constants.MODRINTH_FABRIC_MOD_ID)
                            && installedMod.isFromModrinth()
                            && installedMod.modrinthProject.id
                            .equals(Constants.MODRINTH_QSL_MOD_ID)) {
                            return true;
                        }

                        // don't show Modrinth dependency when grabbed from CurseForge
                        if (dependency.projectId.equals(Constants.MODRINTH_FORGIFIED_FABRIC_API_MOD_ID)
                            && installedMod.isFromCurseForge()
                            && installedMod
                            .getCurseForgeFileId() == Constants.CURSEFORGE_FORGIFIED_FABRIC_API_MOD_ID) {
                            return true;
                        }

                        return installedMod.isFromModrinth()
                            && installedMod.modrinthProject.id.equals(dependency.projectId);
                    }))
                .collect(Collectors.toList());

            if (!dependenciesNeeded.isEmpty()) {
                dependenciesPanel.removeAll();

                dependenciesNeeded.forEach(dependency -> dependenciesPanel
                    .add(new ModrinthProjectDependencyCard(this, dependency, instanceOrServer)));

                dependenciesPanel
                    .setLayout(new GridLayout(dependenciesNeeded.size() < 2 ? 1 : dependenciesNeeded.size() / 2,
                        (dependenciesNeeded.size() / 2) + 1));

                setSize(620, 580);
                setLocationRelativeTo(App.launcher.getParent());

                dependenciesPanel.setVisible(true);

                scrollPane.repaint();
                scrollPane.validate();
            } else {
                setSize(620, 300);
            }
        } else {
            setSize(620, 300);
        }
    }

    private void setupComponents() {
        // #. {0} is the name of the mod we're installing
        setTitle(GetText.tr("Installing {0}", project.title));

        setSize(620, 300);
        setMinimumSize(new Dimension(620, 300));
        setLocationRelativeTo(App.launcher.getParent());
        setResizable(true);
        this.setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);

        addButton = ModernDialogs.primary(GetText.tr("Add"));
        addButton.setEnabled(false);

        viewModButton = ModernDialogs.secondary(GetText.tr("View Mod")).icon(Icons.EXTERNAL);
        if (instanceOrServer.getLoaderVersion() != null && ((instanceOrServer.getLoaderVersion().isPaper()
            && (project.projectType == ModrinthProjectType.PLUGIN || project.loaders.contains("paper")
            || project.loaders.contains("bukkit") || project.loaders.contains("spigot")))
            || (instanceOrServer.getLoaderVersion().isPurpur()
            && (project.projectType == ModrinthProjectType.PLUGIN || project.loaders.contains("purpur")
            || project.loaders.contains("paper")
            || project.loaders.contains("bukkit") || project.loaders.contains("spigot"))))) {
            viewModButton.setText(GetText.tr("View Plugin"));
        }
        viewModButton.setEnabled(false);

        viewFileButton = ModernDialogs.secondary(GetText.tr("View File")).icon(Icons.EXTERNAL);
        viewFileButton.setEnabled(false);

        dependenciesPanel.setVisible(false);
        dependenciesPanel
            .setBorder(BorderFactory.createTitledBorder(GetText.tr("The below mods need to be installed")));

        // Top Panel Stuff
        JPanel top = new JPanel(new BorderLayout());
        top.setOpaque(false);
        // #. {0} is the name of the mod we're installing
        top.add(ModernDialogs.header(GetText.tr("Installing {0}", project.title), null, Icons.DOWNLOAD),
            BorderLayout.NORTH);

        installedJLabel = Lbl.text("", 13f, false, Palette::accent);
        top.add(installedJLabel, BorderLayout.SOUTH);

        // Middle Panel Stuff
        JPanel middle = new JPanel(new BorderLayout());
        middle.setOpaque(false);

        // Middle Panel Stuff
        JPanel versionsPanel = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        versionsPanel.setOpaque(false);
        versionsPanel.setBorder(BorderFactory.createEmptyBorder(6, 0, 6, 0));

        versionsLabel = ModernDialogs.label(GetText.tr("Version To Install") + ": ");
        versionsPanel.add(versionsLabel);

        versionsDropdown = new JComboBox<>();
        ModrinthVersion loadingProject = new ModrinthVersion();
        loadingProject.name = GetText.tr("Loading");
        versionsDropdown.addItem(loadingProject);
        versionsDropdown.setEnabled(false);
        versionsPanel.add(versionsDropdown);

        JPanel filesPanel = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        filesPanel.setOpaque(false);
        filesPanel.setBorder(BorderFactory.createEmptyBorder(6, 0, 6, 0));
        filesLabel = ModernDialogs.label(GetText.tr("File To Install") + ": ");
        filesPanel.add(filesLabel);
        filesDropdown = new JComboBox<>();
        filesPanel.add(filesDropdown);
        filesPanel.setVisible(false);

        scrollPane = new JScrollPane(dependenciesPanel);
        scrollPane.setVerticalScrollBarPolicy(JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED);
        scrollPane.setHorizontalScrollBarPolicy(JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        scrollPane.setPreferredSize(new Dimension(550, 250));
        scrollPane.setBorder(BorderFactory.createEmptyBorder(10, 0, 0, 0));
        scrollPane.setOpaque(false);
        scrollPane.getViewport().setOpaque(false);
        dependenciesPanel.setOpaque(false);

        JPanel selectorPanel = new JPanel();
        selectorPanel.setOpaque(false);
        selectorPanel.setLayout(new BoxLayout(selectorPanel, BoxLayout.Y_AXIS));
        selectorPanel.add(versionsPanel);
        selectorPanel.add(filesPanel);

        middle.add(selectorPanel, BorderLayout.NORTH);
        middle.add(scrollPane, BorderLayout.SOUTH);

        this.getFiles();

        // Bottom Panel Stuff
        addButton.addActionListener(e -> {
            ModrinthVersion version = (ModrinthVersion) versionsDropdown.getSelectedItem();
            ModrinthFile file = filesDropdown.getSelectedItem() == null ? null
                : ((ComboItem<ModrinthFile>) filesDropdown.getSelectedItem()).getValue();

            ProgressDialog<Void> progressDialog = new ProgressDialog<>(
                // #. {0} is the name of the mod we're installing
                GetText.tr("Installing {0}", version.name), true, this);
            progressDialog.addThread(new Thread(() -> {
                com.blockforge.launcher.data.Type effectiveInstallType = getEffectiveInstallType(version);

                Analytics.trackEvent(project.getAnalyticsEventForAdded(version, effectiveInstallType));
                instanceOrServer.addFileFromModrinth(project, version, file, effectiveInstallType, downloadReason,
                    dependentVersionId, progressDialog);
                progressDialog.close();
            }));
            progressDialog.start();
            dispose();
        });

        viewModButton
            .addActionListener(e -> OS.openWebBrowser(String.format("https://modrinth.com/%s/%s",
                getProjectUrlPath(), project.slug)));

        viewFileButton.addActionListener(e -> {
            ModrinthVersion version = (ModrinthVersion) versionsDropdown.getSelectedItem();

            OS.openWebBrowser(String.format("https://modrinth.com/mod/%s/version/%s", project.slug, version.id));
        });

        versionsDropdown.addActionListener(e -> {
            ModrinthVersion version = (ModrinthVersion) versionsDropdown.getSelectedItem();

            if (version != null) {
                reloadDependenciesPanel();

                filesDropdown.removeAllItems();
                if (version.files.size() > 1) {
                    int selectedIndex = 0;

                    for (ModrinthFile file : version.files) {
                        filesDropdown.addItem(new ComboItem<>(file, file.toString()));

                        if (file.primary) {
                            selectedIndex = filesDropdown.getItemCount() - 1;
                        }

                        // ensures that font width is taken into account
                        filesLength = Math.max(filesLength,
                            getFontMetrics(App.THEME.getNormalFont()).stringWidth(file.toString()) + 100);
                    }

                    // ensures that the dropdown is at least 200 px wide and has a maximum width of
                    // 350 px to prevent overflow
                    ModernDialogs.preferWidth(filesDropdown, Math.min(350, Math.max(200, filesLength)));
                    filesDropdown.setSelectedIndex(selectedIndex);
                }

                filesPanel.setVisible(version.files.size() > 1);
            }
        });

        JButton cancel = ModernDialogs.secondary(GetText.tr("Cancel"));
        cancel.addActionListener(e -> dispose());

        ModernDialogs.install(this, top, middle,
            ModernDialogs.footer(viewModButton, viewFileButton, cancel, addButton));
    }

    protected void getFiles() {
        versionsLabel.setVisible(true);
        versionsDropdown.setVisible(true);

        Runnable r = () -> {
            if (this.versionsData == null) {
                this.versionsData = ModrinthApi.getVersions(project.id);
            }

            if (this.versionsData == null) {
                DialogManager.okDialog().setParent(ModrinthVersionSelectorDialog.this).setTitle("No versions found")
                    .setContent("No versions found for this mod").setType(DialogManager.ERROR).show();
                dispose();
                return;
            }

            Stream<ModrinthVersion> modrinthVersionsStream = this.versionsData.stream()
                .sorted(Comparator.comparing((ModrinthVersion version) -> version.datePublished).reversed());

            LoaderVersion loaderVersion = instanceOrServer.getLoaderVersion();
            String minecraftVersion = instanceOrServer.getMinecraftVersion();

            if (App.settings.addModRestriction != AddModRestriction.NONE && loaderVersion != null
                && project.projectType == ModrinthProjectType.MOD
                && this.installType != com.blockforge.launcher.data.Type.datapack) {
                List<String> neoForgeForgeCompatabilityVersions = ConfigManager
                    .getConfigItem("loaders.neoforge.forgeCompatibleMinecraftVersions", new ArrayList<>());
                boolean hasNeoForgeVersion = this.versionsData.stream()
                    .anyMatch(v -> v.loaders.contains("neoforge")
                        || (neoForgeForgeCompatabilityVersions.contains(minecraftVersion)
                        && v.loaders.contains("forge")));
                boolean hasForgeVersion = this.versionsData.stream().anyMatch(v -> v.loaders.contains("forge"));
                modrinthVersionsStream = modrinthVersionsStream.filter(v -> {
                    if (v.loaders.contains("fabric")
                        && (loaderVersion.isFabric()
                        || loaderVersion.isLegacyFabric()
                        || loaderVersion.isQuilt()
                        || (instanceOrServer.isForgeLikeAndHasInstalledSinytraConnector()
                        && loaderVersion.isForge() && !hasForgeVersion)
                        || (instanceOrServer.isForgeLikeAndHasInstalledSinytraConnector()
                        && loaderVersion.isNeoForge() && !hasNeoForgeVersion))) {
                        return true;
                    }

                    if (v.loaders.contains("legacy-fabric") && loaderVersion.isLegacyFabric()) {
                        return true;
                    }

                    if (v.loaders.contains("neoforge") && loaderVersion.isNeoForge()) {
                        return true;
                    }

                    if ((v.loaders.contains("paper") || v.loaders.contains("bukkit") || v.loaders.contains("spigot"))
                        && loaderVersion.isPaper()) {
                        return true;
                    }

                    if ((v.loaders.contains("purpur") || v.loaders.contains("paper") || v.loaders.contains("bukkit")
                        || v.loaders.contains("spigot"))
                        && loaderVersion.isPurpur()) {
                        return true;
                    }

                    if (v.loaders.contains("forge") && (loaderVersion.isForge()
                        || (loaderVersion.isNeoForge()
                        && neoForgeForgeCompatabilityVersions.contains(minecraftVersion)))) {
                        return true;
                    }

                    return v.loaders.contains("quilt") && loaderVersion.isQuilt();
                });
            }

            if (this.installType == com.blockforge.launcher.data.Type.datapack) {
                modrinthVersionsStream = modrinthVersionsStream
                    .filter(v -> v.loaders != null && v.loaders.contains("datapack"));
            }

            if (App.settings.addModRestriction == AddModRestriction.STRICT) {
                modrinthVersionsStream = modrinthVersionsStream.filter(v -> v.gameVersions.contains(minecraftVersion));
            } else if (App.settings.addModRestriction == AddModRestriction.LAX) {
                try {
                    List<String> minecraftVersionsToSearch = MinecraftManager
                        .getMajorMinecraftVersions(minecraftVersion).stream().map(mv -> mv.id)
                        .collect(Collectors.toList());

                    modrinthVersionsStream = modrinthVersionsStream.filter(
                        v -> v.gameVersions.stream().anyMatch(minecraftVersionsToSearch::contains));
                } catch (InvalidMinecraftVersion e) {
                    LogManager.logStackTrace(e);
                }
            }

            versions.addAll(modrinthVersionsStream.collect(Collectors.toList()));

            // ensures that font width is taken into account
            for (ModrinthVersion version : versions) {
                versionsLength = Math.max(versionsLength,
                    getFontMetrics(App.THEME.getNormalFont()).stringWidth(version.name) + 100);
            }

            versionsDropdown.removeAllItems();

            versions.forEach(version -> versionsDropdown.addItem(version));

            if (versionsDropdown.getItemCount() == 0) {
                DialogManager.okDialog().setParent(ModrinthVersionSelectorDialog.this).setTitle("No versions found")
                    .setContent("No versions found for this mod").setType(DialogManager.ERROR).show();
                dispose();
            }

            if (this.installedVersionId != null) {
                ModrinthVersion installedFile = versions.stream()
                    .filter(f -> f.id.equalsIgnoreCase(this.installedVersionId)).findFirst().orElse(null);

                if (installedFile != null) {
                    if (!selectNewest) {
                        versionsDropdown.setSelectedItem(installedFile);
                    }

                    // #. {0} is the name of the mod that the user already has installed
                    installedJLabel.setText(GetText.tr("The version currently installed is {0}", installedFile.name));
                    installedJLabel.setVisible(true);
                }
            }

            // ensures that the dropdown is at least 200 px wide and has a maximum width of
            // 350 px to prevent overflow
            ModernDialogs.preferWidth(versionsDropdown, Math.min(350, Math.max(200, versionsLength)));

            versionsDropdown.setEnabled(true);
            versionsLabel.setVisible(true);
            versionsDropdown.setVisible(true);
            addButton.setEnabled(true);
            viewModButton.setEnabled(true);
            viewFileButton.setEnabled(true);
        };

        new Thread(r).start();
    }
}
