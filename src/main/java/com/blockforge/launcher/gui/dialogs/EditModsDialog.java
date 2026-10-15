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
import java.awt.Color;
import java.awt.Dimension;
import java.awt.GridLayout;
import java.awt.event.ItemEvent;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.io.File;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.stream.Collectors;

import javax.swing.AbstractButton;
import javax.swing.BorderFactory;
import javax.swing.BoxLayout;
import javax.swing.JButton;
import javax.swing.JCheckBox;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JLayer;
import javax.swing.JList;
import javax.swing.JPanel;
import javax.swing.JScrollPane;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.ModManagement;
import com.blockforge.launcher.data.Server;
import com.blockforge.launcher.data.curseforge.CurseForgeFingerprint;
import com.blockforge.launcher.data.curseforge.CurseForgeProject;
import com.blockforge.launcher.data.minecraft.FabricMod;
import com.blockforge.launcher.data.minecraft.MCMod;
import com.blockforge.launcher.data.modrinth.ModrinthProject;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.gui.WheelScrollLayerUI;
import com.blockforge.launcher.gui.components.ModsJCheckBox;
import com.blockforge.launcher.gui.handlers.ModsJCheckBoxTransferHandler;
import com.blockforge.launcher.gui.layouts.WrapLayout;
import com.blockforge.launcher.gui.modern.Btn;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.managers.ConfigManager;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.utils.CurseForgeApi;
import com.blockforge.launcher.utils.FileUtils;
import com.blockforge.launcher.utils.Hashing;
import com.blockforge.launcher.utils.ModrinthApi;
import com.blockforge.launcher.utils.Utils;

public class EditModsDialog extends JDialog {
    private static final long serialVersionUID = 7004414192679481818L;

    public final ModManagement instanceOrServer;

    private JPanel disabledModsPanel, enabledModsPanel;
    private JButton checkForUpdatesButton;
    private JButton reinstallButton;
    private JButton enableButton;
    private JButton disableButton;
    private JButton removeButton;
    private JButton refreshMetadataButton;
    private JCheckBox selectAllEnabledModsCheckbox, selectAllDisabledModsCheckbox;
    private ArrayList<ModsJCheckBox> enabledMods, disabledMods;

    public EditModsDialog(Instance instance) {
        super(App.launcher.getParent(),
            // #. {0} is the name of the instance
            GetText.tr("Editing Mods For {0}", instance.launcher.name), ModalityType.DOCUMENT_MODAL);
        this.instanceOrServer = instance;

        setup();
    }

    public EditModsDialog(Server server) {
        super(App.launcher.getParent(),
            // #. {0} is the name of the instance
            GetText.tr("Editing Mods For {0}", server.name), ModalityType.DOCUMENT_MODAL);
        this.instanceOrServer = server;

        setup();
    }

    private void setup() {
        setSize(780, 580);
        setMinimumSize(new Dimension(640, 480));
        setLocationRelativeTo(App.launcher.getParent());
        setResizable(true);
        setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);
        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosing(WindowEvent arg0) {
                dispose();
            }
        });

        setupComponents();

        instanceOrServer.scanMissingMods(this);

        loadMods();
    }

    private void setupComponents() {
        Analytics.sendScreenView("Edit Mods Dialog");

        JPanel topLeftPanel = new JPanel(new BorderLayout());
        topLeftPanel.setOpaque(false);

        JLabel topLabelLeft = Lbl.text(GetText.tr("Enabled Mods"), 14f, true, Palette::text);
        topLeftPanel.add(topLabelLeft, BorderLayout.CENTER);

        selectAllEnabledModsCheckbox = new JCheckBox();
        selectAllEnabledModsCheckbox.setOpaque(false);
        selectAllEnabledModsCheckbox.addActionListener(e -> {
            boolean selected = selectAllEnabledModsCheckbox.isSelected();

            enabledMods.forEach(em -> em.setSelected(selected));
        });
        topLeftPanel.add(selectAllEnabledModsCheckbox, BorderLayout.EAST);

        JPanel topRightPanel = new JPanel(new BorderLayout());
        topRightPanel.setOpaque(false);

        JLabel topLabelRight = Lbl.text(GetText.tr("Disabled Mods"), 14f, true, Palette::text);
        topRightPanel.add(topLabelRight, BorderLayout.CENTER);

        selectAllDisabledModsCheckbox = new JCheckBox();
        selectAllDisabledModsCheckbox.setOpaque(false);
        selectAllDisabledModsCheckbox.addActionListener(e -> {
            boolean selected = selectAllDisabledModsCheckbox.isSelected();

            disabledMods.forEach(dm -> dm.setSelected(selected));
        });
        topRightPanel.add(selectAllDisabledModsCheckbox, BorderLayout.EAST);

        JPanel modsInPack = new JPanel(new GridLayout(1, 2, 14, 0));
        modsInPack.setOpaque(false);

        disabledModsPanel = new JPanel() {
            @Override
            public Color getBackground() {
                return Palette.card();
            }
        };
        disabledModsPanel.setLayout(new BoxLayout(disabledModsPanel, BoxLayout.Y_AXIS));
        disabledModsPanel.setTransferHandler(new ModsJCheckBoxTransferHandler(this, true));

        JScrollPane scroller1 = new JScrollPane(disabledModsPanel,
            JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
            JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        scroller1.getVerticalScrollBar().setUnitIncrement(16);

        enabledModsPanel = new JPanel() {
            @Override
            public Color getBackground() {
                return Palette.card();
            }
        };
        enabledModsPanel.setLayout(new BoxLayout(enabledModsPanel, BoxLayout.Y_AXIS));
        enabledModsPanel.setTransferHandler(new ModsJCheckBoxTransferHandler(this, true));;

        JScrollPane scroller2 = new JScrollPane(enabledModsPanel,
            JScrollPane.VERTICAL_SCROLLBAR_ALWAYS,
            JScrollPane.HORIZONTAL_SCROLLBAR_NEVER);
        scroller2.getVerticalScrollBar().setUnitIncrement(16);

        modsInPack.add(ModernDialogs.listCard(topLeftPanel, scroller2, new JLayer<>(scroller2, new WheelScrollLayerUI())));
        modsInPack.add(ModernDialogs.listCard(topRightPanel, scroller1, new JLayer<>(scroller1, new WheelScrollLayerUI())));

        JPanel bottomPanel = new JPanel(new WrapLayout(WrapLayout.RIGHT, 8, 8));
        bottomPanel.setOpaque(false);
        bottomPanel.setBorder(BorderFactory.createEmptyBorder(10, 0, 0, 0));

        ModernDialogs.install(this, ModernDialogs.header(getTitle(), null, Icons.CUBE), modsInPack, bottomPanel);

        JButton addButton = new Btn(GetText.tr("Add Mod"), Btn.Kind.TONAL).icon(Icons.PLUS).compact();
        addButton.addActionListener(e -> {
            String[] modTypes;

            if (instanceOrServer instanceof Instance) {
                modTypes = new String[] { "Mods Folder", "Data Pack", "Resource Pack", "Shader Pack",
                    "Inside Minecraft.jar" };
            } else if (instanceOrServer.getLoaderVersion() != null && (instanceOrServer.getLoaderVersion().isPaper()
                || instanceOrServer.getLoaderVersion().isPurpur())) {
                modTypes = new String[] { "Plugins Folder" };
            } else {
                modTypes = new String[] { "Mods Folder" };
            }

            FileChooserDialog fcd = new FileChooserDialog(this, GetText.tr("Add Mod"), GetText.tr("Mod"),
                GetText.tr("Add"), GetText.tr("Type of Mod"), modTypes);
            fcd.setVisible(true);

            if (fcd.wasClosed()) {
                return;
            }

            final ProgressDialog<Object> progressDialog = new ProgressDialog<>(GetText.tr("Copying Mods"), 0,
                GetText.tr("Copying Mods"), this);

            progressDialog.addThread(new Thread(() -> {
                List<File> files = fcd.getChosenFiles();
                if (files != null && !files.isEmpty()) {
                    boolean reload = false;
                    for (File file : files) {
                        String typeTemp = fcd.getSelectorValue();
                        com.blockforge.launcher.data.Type type = null;
                        if (typeTemp.equalsIgnoreCase("Mods Folder")) {
                            type = com.blockforge.launcher.data.Type.mods;
                        } else if (typeTemp.equalsIgnoreCase("Inside Minecraft.jar")) {
                            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Add As Mod?"))
                                .setContent(new HTMLBuilder().text(GetText.tr(
                                        "Adding as Inside Minecraft.jar is usually not what you want and will likely cause issues.<br/><br/>If you're adding mods this is usually not correct. Do you want to add this as a mod instead?"))
                                    .build())
                                .setType(DialogManager.WARNING).show();

                            if (ret != 0) {
                                type = com.blockforge.launcher.data.Type.jar;
                            } else {
                                type = com.blockforge.launcher.data.Type.mods;
                            }
                        } else if (typeTemp.equalsIgnoreCase("CoreMods Mod")) {
                            type = com.blockforge.launcher.data.Type.coremods;
                        } else if (typeTemp.equalsIgnoreCase("Texture Pack")) {
                            type = com.blockforge.launcher.data.Type.texturepack;
                        } else if (typeTemp.equalsIgnoreCase("Data Pack")) {
                            type = com.blockforge.launcher.data.Type.datapack;
                        } else if (typeTemp.equalsIgnoreCase("Resource Pack")) {
                            type = com.blockforge.launcher.data.Type.resourcepack;
                        } else if (typeTemp.equalsIgnoreCase("Shader Pack")) {
                            type = com.blockforge.launcher.data.Type.shaderpack;
                        } else if (typeTemp.equalsIgnoreCase("Plugins Folder")) {
                            type = com.blockforge.launcher.data.Type.plugins;
                        }
                        if (type != null) {
                            DisableableMod mod = DisableableMod.generateMod(file, type,
                                App.settings.enableAddedModsByDefault);
                            File copyTo = App.settings.enableAddedModsByDefault ? mod.getFile(instanceOrServer)
                                : mod.getDisabledFile(instanceOrServer);

                            if (copyTo.exists()) {
                                LogManager.warn("The file " + file.getName() + " already exists. Not adding!");
                                continue;
                            }

                            if (!copyTo.getParentFile().exists()) {
                                copyTo.getParentFile().mkdirs();
                            }

                            if (Utils.copyFile(file, copyTo, true)) {
                                instanceOrServer.addMod(mod);
                                reload = true;
                            }
                        }
                    }
                    if (reload) {
                        reloadPanels();
                    }
                }
                progressDialog.close();
            }));

            progressDialog.start();
        });
        bottomPanel.add(addButton);

        if (instanceOrServer instanceof Server || (instanceOrServer instanceof Instance
            && ((Instance) instanceOrServer).launcher.enableCurseForgeIntegration)) {
            if (ConfigManager.getConfigItem("platforms.curseforge.modsEnabled", true)
                || (ConfigManager.getConfigItem("platforms.modrinth.modsEnabled", true)
                && (instanceOrServer.getLoaderVersion() != null
                || instanceOrServer instanceof Instance))) {
                JButton browseMods = new Btn(GetText.tr("Browse Mods"), Btn.Kind.FILLED).icon(Icons.SEARCH).compact();
                browseMods.addActionListener(e -> {
                    AddModsDialog addModsDialog = new AddModsDialog(this, instanceOrServer);
                    addModsDialog.setVisible(true);

                    loadMods();

                    reloadPanels();
                });
                bottomPanel.add(browseMods);
            }

            checkForUpdatesButton = new Btn(GetText.tr("Check For Updates"), Btn.Kind.TONAL).compact();
            checkForUpdatesButton.addActionListener(e -> checkForUpdates());
            checkForUpdatesButton.setEnabled(false);
            bottomPanel.add(checkForUpdatesButton);

            reinstallButton = new Btn(GetText.tr("Reinstall"), Btn.Kind.TONAL).compact();
            reinstallButton.addActionListener(e -> reinstall());
            reinstallButton.setEnabled(false);
            bottomPanel.add(reinstallButton);
        }

        enableButton = new Btn(GetText.tr("Enable Selected"), Btn.Kind.TONAL).compact();
        enableButton.addActionListener(e -> enableMods());
        enableButton.setEnabled(false);
        bottomPanel.add(enableButton);

        disableButton = new Btn(GetText.tr("Disable Selected"), Btn.Kind.TONAL).compact();
        disableButton.addActionListener(e -> disableMods());
        disableButton.setEnabled(false);
        bottomPanel.add(disableButton);

        removeButton = new Btn(GetText.tr("Remove Selected"), Btn.Kind.DANGER).compact();
        removeButton.addActionListener(e -> removeMods());
        removeButton.setEnabled(false);
        bottomPanel.add(removeButton);

        refreshMetadataButton = new Btn(GetText.tr("Refresh Metadata"), Btn.Kind.TONAL).compact();
        refreshMetadataButton.addActionListener(e -> refreshMetadata());
        refreshMetadataButton.setEnabled(false);
        bottomPanel.add(refreshMetadataButton);
    }

    private void loadMods() {
        List<DisableableMod> mods = instanceOrServer.getMods().stream().filter(DisableableMod::wasSelected)
            .filter(m -> !m.skipped && m.type != com.blockforge.launcher.data.Type.worlds)
            .sorted(Comparator.comparing(m -> m.name, String.CASE_INSENSITIVE_ORDER)).collect(Collectors.toList());
        enabledMods = new ArrayList<>();
        disabledMods = new ArrayList<>();
        int dCount = 0;
        int eCount = 0;

        for (DisableableMod mod : mods) {
            ModsJCheckBox checkBox;
            int nameSize = getFontMetrics(App.THEME.getNormalFont()).stringWidth(mod.getName());

            checkBox = new ModsJCheckBox(mod, this);
            if (mod.isDisabled()) {
                checkBox.setBounds(0, (dCount * 20), Math.max(nameSize + 23, 250), 20);
                disabledMods.add(checkBox);
                dCount++;
            } else {
                checkBox.setBounds(0, (eCount * 20), Math.max(nameSize + 23, 250), 20);
                enabledMods.add(checkBox);
                eCount++;
            }
        }

        for (ModsJCheckBox checkBox : enabledMods) {
            checkBox.addItemListener(e -> {
                if (e.getStateChange() == ItemEvent.SELECTED || e.getStateChange() == ItemEvent.DESELECTED) {
                    checkBoxesChanged();
                }
            });
            enabledModsPanel.add(checkBox);
        }
        for (ModsJCheckBox checkBox : disabledMods) {
            checkBox.addItemListener(e -> {
                if (e.getStateChange() == ItemEvent.SELECTED || e.getStateChange() == ItemEvent.DESELECTED) {
                    checkBoxesChanged();
                }
            });
            disabledModsPanel.add(checkBox);
        }
        int enabledHeight = enabledMods.stream()
                .mapToInt(cb -> cb.getPreferredSize().height)
                .sum();
        enabledModsPanel.setPreferredSize(new Dimension(0, enabledHeight));

        int disabledHeight = disabledMods.stream()
                .mapToInt(cb -> cb.getPreferredSize().height)
                .sum();
        disabledModsPanel.setPreferredSize(new Dimension(0, disabledHeight));
    }

    private void checkBoxesChanged() {
        if (instanceOrServer instanceof Server || (instanceOrServer instanceof Instance
            && ((Instance) instanceOrServer).launcher.enableCurseForgeIntegration)) {
            boolean hasSelectedACurseForgeOrModrinthMod = (enabledMods.stream().anyMatch(AbstractButton::isSelected)
                && enabledMods.stream().filter(AbstractButton::isSelected)
                .anyMatch(cb -> cb.getDisableableMod().isUpdatable()))
                || (disabledMods.stream().anyMatch(AbstractButton::isSelected) && disabledMods.stream()
                .filter(AbstractButton::isSelected).anyMatch(cb -> cb.getDisableableMod().isUpdatable()));

            checkForUpdatesButton.setEnabled(hasSelectedACurseForgeOrModrinthMod);
            reinstallButton.setEnabled(hasSelectedACurseForgeOrModrinthMod);
        }

        removeButton.setEnabled((!disabledMods.isEmpty() && disabledMods.stream().anyMatch(AbstractButton::isSelected))
            || (!enabledMods.isEmpty() && enabledMods.stream().anyMatch(AbstractButton::isSelected)));
        enableButton.setEnabled(!disabledMods.isEmpty() && disabledMods.stream().anyMatch(AbstractButton::isSelected));
        disableButton.setEnabled(!enabledMods.isEmpty() && enabledMods.stream().anyMatch(AbstractButton::isSelected));
        refreshMetadataButton
            .setEnabled(!enabledMods.isEmpty() && enabledMods.stream().anyMatch(AbstractButton::isSelected));

        selectAllEnabledModsCheckbox
            .setSelected(!enabledMods.isEmpty() && enabledMods.stream().allMatch(AbstractButton::isSelected));
        selectAllDisabledModsCheckbox
            .setSelected(!disabledMods.isEmpty() && disabledMods.stream().allMatch(AbstractButton::isSelected));
    }

    private void checkForUpdates() {
        ArrayList<ModsJCheckBox> mods = new ArrayList<>();
        mods.addAll(enabledMods);
        mods.addAll(disabledMods);

        ProgressDialog<Void> progressDialog = new ProgressDialog<>(GetText.tr("Checking For Updates"), mods.size(),
            GetText.tr("Checking For Updates"), this);
        progressDialog.addThread(new Thread(() -> {
            for (ModsJCheckBox mod : mods) {
                if (mod.isSelected() && mod.getDisableableMod().isUpdatable()) {
                    mod.getDisableableMod().checkForUpdate(progressDialog, instanceOrServer);
                }
                progressDialog.doneTask();
            }

            progressDialog.close();
        }));
        progressDialog.start();

        DialogManager.okDialog().setTitle(GetText.tr("Checking For Updates Complete"))
            .setContent(GetText.tr("The selected mods have been checked for updates.")).show();

        reloadPanels();
    }

    private void reinstall() {
        ArrayList<ModsJCheckBox> mods = new ArrayList<>();
        mods.addAll(enabledMods);
        mods.addAll(disabledMods);

        for (ModsJCheckBox mod : mods) {
            if (mod.isSelected() && mod.getDisableableMod().isUpdatable()) {
                mod.getDisableableMod().reinstall(this, instanceOrServer);
            }
        }
        reloadPanels();
    }

    private void enableMods() {
        ArrayList<ModsJCheckBox> mods = new ArrayList<>(disabledMods);
        for (ModsJCheckBox mod : mods) {
            if (mod.isSelected()) {
                mod.getDisableableMod().enable(instanceOrServer);
            }
        }
        reloadPanels();
    }

    private void disableMods() {
        ArrayList<ModsJCheckBox> mods = new ArrayList<>(enabledMods);
        for (ModsJCheckBox mod : mods) {
            if (mod.isSelected()) {
                mod.getDisableableMod().disable(instanceOrServer);
            }
        }
        reloadPanels();
    }

    private void removeMods() {
        int ret = DialogManager.yesNoDialog(false)
            .setTitle(GetText.tr("Delete Selected Mods?"))
            .setContent(new HTMLBuilder().center().text(GetText.tr(
                    "This will delete the selected mods from the instance.<br/><br/>Are you sure you want to do this?"))
                .build())
            .setType(DialogManager.WARNING).show();

        if (ret == 0) {
            ArrayList<ModsJCheckBox> mods = new ArrayList<>(enabledMods);
            for (ModsJCheckBox mod : mods) {
                if (mod.isSelected()) {
                    instanceOrServer.getMods().remove(mod.getDisableableMod());
                    FileUtils.delete(
                        (mod.getDisableableMod().isDisabled()
                            ? mod.getDisableableMod().getDisabledFile(instanceOrServer)
                            : mod.getDisableableMod().getFile(instanceOrServer)).toPath(),
                        true);
                    enabledMods.remove(mod);
                }
            }
            mods = new ArrayList<>(disabledMods);
            for (ModsJCheckBox mod : mods) {
                if (mod.isSelected()) {
                    instanceOrServer.getMods().remove(mod.getDisableableMod());
                    FileUtils.delete(
                        (mod.getDisableableMod().isDisabled()
                            ? mod.getDisableableMod().getDisabledFile(instanceOrServer)
                            : mod.getDisableableMod().getFile(instanceOrServer)).toPath(),
                        true);
                    disabledMods.remove(mod);
                }
            }
            reloadPanels();
        }
    }

    private void refreshMetadata() {
        final ProgressDialog<Boolean> dialog = new ProgressDialog<>(GetText.tr("Refreshing Metadata"), 0,
            GetText.tr("Refreshing Metadata"),
            "Aborting refreshing metadata");
        dialog.addThread(new Thread(() -> {

            List<ModsJCheckBox> modsToRefresh = new ArrayList<>();
            modsToRefresh
                .addAll(enabledMods.parallelStream().filter(ModsJCheckBox::isSelected)
                    .collect(Collectors.toList()));
            modsToRefresh
                .addAll(disabledMods.parallelStream().filter(ModsJCheckBox::isSelected)
                    .collect(Collectors.toList()));

            // TODO: Generalise this, cause fuck me I've copy pasted this like 10 times now
            if (!App.settings.dontCheckModsOnCurseForge) {
                Map<Long, ModsJCheckBox> murmurHashes = new HashMap<>();

                modsToRefresh.stream()
                    .filter(mjc -> mjc.getDisableableMod().getFile(instanceOrServer.getRoot(),
                        instanceOrServer.getMinecraftVersion()) != null)
                    .forEach(mjc -> {
                        try {
                            long hash = Hashing
                                .murmur(mjc.getDisableableMod().getFile(instanceOrServer.getRoot(),
                                    instanceOrServer.getMinecraftVersion()).toPath());
                            murmurHashes.put(hash, mjc);
                        } catch (IOException t) {
                            LogManager.logStackTrace(t);
                        }
                    });

                if (!murmurHashes.isEmpty()) {
                    CurseForgeFingerprint fingerprintResponse = CurseForgeApi
                        .checkFingerprints(murmurHashes.keySet().stream().toArray(Long[]::new));

                    if (fingerprintResponse != null && fingerprintResponse.exactMatches != null) {
                        int[] projectIdsFound = fingerprintResponse.exactMatches.stream().mapToInt(em -> em.id)
                            .toArray();

                        if (projectIdsFound.length != 0) {
                            Map<Integer, CurseForgeProject> foundProjects = CurseForgeApi
                                .getProjectsAsMap(projectIdsFound);

                            if (foundProjects != null) {
                                fingerprintResponse.exactMatches.stream().filter(em -> em != null && em.file != null
                                    && murmurHashes.containsKey(em.file.packageFingerprint)).forEach(foundMod -> {
                                    DisableableMod dm = murmurHashes.get(foundMod.file.packageFingerprint)
                                        .getDisableableMod();

                                    CurseForgeProject curseForgeProject = foundProjects.get(foundMod.id);

                                    if (curseForgeProject != null && curseForgeProject.status == 4) {
                                        dm.curseForgeProjectId = foundMod.id;
                                        dm.curseForgeFile = foundMod.file;
                                        dm.curseForgeFileId = foundMod.file.id;
                                        dm.curseForgeProject = curseForgeProject;
                                        dm.name = curseForgeProject.name;
                                        dm.description = curseForgeProject.summary;

                                        LogManager.debug("Found matching mod from CurseForge called "
                                            + dm.curseForgeFile.displayName);
                                    }

                                    // reset if the file is not approved
                                    if (curseForgeProject != null && curseForgeProject.status != 4) {
                                        dm.curseForgeProjectId = null;
                                        dm.curseForgeFile = null;
                                        dm.curseForgeFileId = null;
                                        dm.curseForgeProject = null;

                                        File path = dm.getFile(instanceOrServer);
                                        MCMod mcMod = Utils.getMCModForFile(path);
                                        if (mcMod != null) {
                                            dm.name = Optional.ofNullable(mcMod.name)
                                                .orElse(path.getName());
                                            dm.description = mcMod.description;
                                        } else {
                                            FabricMod fabricMod = Utils.getFabricModForFile(path);
                                            if (fabricMod != null) {
                                                dm.name = Optional.ofNullable(fabricMod.name)
                                                    .orElse(path.getName());
                                                dm.description = fabricMod.description;
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            }

            if (!App.settings.dontCheckModsOnModrinth) {
                Map<String, ModsJCheckBox> sha1Hashes = new HashMap<>();

                modsToRefresh.stream()
                    .filter(mjc -> mjc.getDisableableMod().getFile(instanceOrServer.getRoot(),
                        instanceOrServer.getMinecraftVersion()) != null)
                    .forEach(mjc -> {
                        try {
                            sha1Hashes.put(
                                Hashing.sha1(
                                        mjc.getDisableableMod()
                                            .getFile(instanceOrServer.getRoot(),
                                                instanceOrServer.getMinecraftVersion())
                                            .toPath())
                                    .toString(),
                                mjc);
                        } catch (Throwable t) {
                            LogManager.logStackTrace(t);
                        }
                    });

                if (!sha1Hashes.isEmpty()) {
                    Set<String> keys = sha1Hashes.keySet();
                    Map<String, ModrinthVersion> modrinthVersions = ModrinthApi
                        .getVersionsFromSha1Hashes(keys.toArray(new String[0]));

                    if (modrinthVersions != null && !modrinthVersions.isEmpty()) {
                        String[] projectIdsFound = modrinthVersions.values().stream().map(mv -> mv.projectId)
                            .toArray(String[]::new);

                        if (projectIdsFound.length != 0) {
                            Map<String, ModrinthProject> foundProjects = ModrinthApi.getProjectsAsMap(projectIdsFound);

                            if (foundProjects != null) {
                                for (Map.Entry<String, ModrinthVersion> entry : modrinthVersions.entrySet()) {
                                    ModrinthVersion version = entry.getValue();
                                    ModrinthProject project = foundProjects.get(version.projectId);

                                    if (project != null) {
                                        DisableableMod dm = sha1Hashes.get(entry.getKey()).getDisableableMod();

                                        // add Modrinth information
                                        dm.modrinthProject = project;
                                        dm.modrinthVersion = version;
                                        dm.name = project.title;
                                        dm.description = project.description;

                                        LogManager
                                            .debug(String.format(
                                                "Found matching mod from Modrinth called %s with file %s",
                                                project.title, version.name));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            instanceOrServer.save();

            dialog.close();
        }));
        dialog.start();

        reloadPanels();
    }

    public void reloadPanels() {
        instanceOrServer.save();

        enabledModsPanel.removeAll();
        disabledModsPanel.removeAll();
        loadMods();
        checkBoxesChanged();
        enabledModsPanel.revalidate();
        enabledModsPanel.repaint();
        disabledModsPanel.revalidate();
        disabledModsPanel.repaint();
    }

}