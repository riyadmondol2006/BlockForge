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
package com.blockforge.launcher.gui.dialogs.instancesettings;

import java.awt.BorderLayout;
import java.awt.FlowLayout;
import java.awt.event.ItemEvent;
import java.io.File;
import java.util.List;
import java.util.Map;
import java.util.Optional;

import javax.swing.JButton;
import javax.swing.JComboBox;
import javax.swing.JFileChooser;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JSpinner;
import javax.swing.JTextArea;
import javax.swing.JTextField;
import javax.swing.ScrollPaneConstants;
import javax.swing.SpinnerNumberModel;
import javax.swing.SwingUtilities;
import javax.swing.text.AbstractDocument;
import javax.swing.text.AttributeSet;
import javax.swing.text.BadLocationException;
import javax.swing.text.DocumentFilter;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.Data;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.JavaRuntime;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.utils.ComboItem;
import com.blockforge.launcher.utils.Java;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.javafinder.JavaInfo;
import com.formdev.flatlaf.ui.FlatScrollPaneBorder;

public class JavaInstanceSettingsTab extends JPanel {
    private final Instance instance;

    private JSpinner maximumMemory;
    private JSpinner permGen;
    private JTextField javaPath;
    private JTextArea javaParameters;
    private JComboBox<ComboItem<String>> javaRuntimeOverride;
    private JComboBox<ComboItem<Boolean>> useJavaProvidedByMinecraft;
    private JComboBox<ComboItem<Boolean>> disableLegacyLaunching;
    private JComboBox<ComboItem<Boolean>> useSystemGlfw;
    private JComboBox<ComboItem<Boolean>> useSystemOpenAl;

    private boolean permgenWarningShown = false;

    public JavaInstanceSettingsTab(Instance instance) {
        this.instance = instance;

        setupComponents();
    }

    private void setupComponents() {
        int systemRam = OS.getSystemRam();
        SettingRows.column(this);

        // Maximum Memory Settings
        SpinnerNumberModel maximumMemoryModel = new SpinnerNumberModel(
            getIfNotNull(this.instance.launcher.maximumMemory, App.settings.maximumMemory), null, null, 512);
        maximumMemoryModel.setMinimum(512);
        maximumMemoryModel.setMaximum((systemRam == 0 ? null : systemRam));
        maximumMemory = new JSpinner(maximumMemoryModel);
        JPanel maximumMemoryRow = SettingRows.row(GetText.tr("Maximum memory"),
            systemRam == 0 ? GetText.tr("The most memory Minecraft may use")
                : GetText.tr("The most memory Minecraft may use. This computer has {0}",
                    SettingRows.gigabytes(systemRam)),
            SettingRows.memory(maximumMemory, 512, systemRam == 0 ? 16384 : systemRam));

        // Perm Gen Settings
        SpinnerNumberModel permGenModel = new SpinnerNumberModel(
            getIfNotNull(this.instance.launcher.permGen, App.settings.metaspace), null, null, 32);
        permGenModel.setMinimum(32);
        permGenModel.setMaximum((systemRam == 0 ? null : systemRam));
        permGen = new JSpinner(permGenModel);
        permGen.addChangeListener(e -> {
            JSpinner s = (JSpinner) e.getSource();
            int permGenMaxRecommendedSize = (OS.is64Bit() ? 256 : 128);

            if ((Integer) s.getValue() > permGenMaxRecommendedSize && !permgenWarningShown) {
                permgenWarningShown = true;
                int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Warning"))
                    .setType(DialogManager.WARNING)
                    .setContent(GetText.tr(
                        "Setting PermGen size above {0}MB is not recommended and can cause issues. Are you sure you want to do this?",
                        permGenMaxRecommendedSize))
                    .show();

                if (ret != 0) {
                    permGen.setValue(permGenMaxRecommendedSize);
                }
            }
        });
        JPanel permGenRow = SettingRows.row(GetText.tr("PermGen size"),
            GetText.tr("Memory for loaded code, only used by Java 7 and by older modpacks that ask for it"),
            SettingRows.withUnit(permGen, GetText.tr("MB")));
        // only old Java and old packs need this, so it's left out unless it matters for this instance
        permGenRow.setVisible(needsPermGen());

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Memory"), maximumMemoryRow, permGenRow));

        // Java Path
        javaPath = new JTextField(32);

        final JLabel javaPathDummy = Lbl.text(GetText.tr("Provided by Minecraft"), 13f, true, Palette::textDim);
        JPanel javaMinecraftProvidedRow = SettingRows.row(GetText.tr("Java"),
            GetText.tr("This Minecraft version comes with its own Java. To pick another, turn off \"Use Java provided by Minecraft\" (not recommended)"),
            javaPathDummy);

        JPanel javaPathPanel = new JPanel(new BorderLayout(0, 8));
        javaPathPanel.setOpaque(false);

        JPanel javaPathPanelBottom = new JPanel(new BorderLayout(8, 0));
        javaPathPanelBottom.setOpaque(false);

        javaPath.setText(getIfNotNull(this.instance.launcher.javaPath, App.settings.javaPath));
        JButton javaPathResetButton = ModernDialogs.secondary(GetText.tr("Reset")).compact();
        javaPathResetButton.addActionListener(e -> javaPath.setText(OS.getDefaultJavaPath()));
        JButton javaBrowseButton = ModernDialogs.secondary(GetText.tr("Browse")).compact();
        javaBrowseButton.addActionListener(e -> {
            JFileChooser chooser = new JFileChooser();
            chooser.setCurrentDirectory(new File(javaPath.getText()));
            chooser.setDialogTitle(GetText.tr("Select path to Java install"));
            chooser.setFileSelectionMode(JFileChooser.DIRECTORIES_ONLY);
            chooser.setAcceptAllFileFilterUsed(false);

            if (chooser.showOpenDialog(null) == JFileChooser.APPROVE_OPTION) {
                File selectedPath = chooser.getSelectedFile();
                File jPath = new File(selectedPath, "bin");
                File javaExe = new File(selectedPath, "java.exe");
                File javaExecutable = new File(selectedPath, "java");

                // user selected the bin dir
                if (!jPath.exists() && (javaExe.exists() || javaExecutable.exists())) {
                    javaPath.setText(selectedPath.getParent());
                } else {
                    javaPath.setText(selectedPath.getAbsolutePath());
                }
            }
        });

        JComboBox<ComboItem<JavaInfo>> installedJavasComboBox = new JComboBox<>();
        installedJavasComboBox.addItem(new ComboItem<>(null, GetText.tr("Use launcher default")));
        List<JavaInfo> installedJavas = Java.getInstalledJavas();
        int selectedIndex = 0;

        for (JavaInfo javaInfo : installedJavas) {
            installedJavasComboBox.addItem(new ComboItem<>(javaInfo, javaInfo.toString()));

            if (javaInfo.rootPath
                .equalsIgnoreCase(getIfNotNull(this.instance.launcher.javaPath, App.settings.javaPath))) {
                selectedIndex = installedJavasComboBox.getItemCount() - 2;
            }
        }
        // installed Javas are named by their (long) paths, so let the dropdown shrink to the window
        installedJavasComboBox.setPrototypeDisplayValue(new ComboItem<>(null, "Java"));

        if (installedJavasComboBox.getItemCount() != 1) {
            installedJavasComboBox.setSelectedIndex(selectedIndex);
            installedJavasComboBox.addActionListener(e -> {
                JavaInfo selectedItem = ((ComboItem<JavaInfo>) installedJavasComboBox.getSelectedItem()).getValue();

                if (selectedItem == null) {
                    javaPath.setText("");
                } else {
                    javaPath.setText(selectedItem.rootPath);
                }
            });

            javaPathPanel.add(installedJavasComboBox, BorderLayout.NORTH);
        }

        JPanel javaPathButtons = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
        javaPathButtons.setOpaque(false);
        javaPathButtons.add(javaPathResetButton);
        javaPathButtons.add(javaBrowseButton);
        javaPath.setColumns(1);
        javaPathPanelBottom.add(javaPath, BorderLayout.CENTER);
        javaPathPanelBottom.add(javaPathButtons, BorderLayout.EAST);

        javaPathPanel.add(javaPathPanelBottom, BorderLayout.CENTER);

        JPanel javaPathRow = SettingRows.stacked(GetText.tr("Java path"),
            GetText.tr("The folder that holds Java's bin folder. Best left as it is; Reset goes back to the default"),
            javaPathPanel);

        boolean isUsingMinecraftProvidedJava = Optional.ofNullable(instance.launcher.useJavaProvidedByMinecraft)
            .orElse(App.settings.useJavaProvidedByMinecraft);
        javaMinecraftProvidedRow.setVisible(instance.javaVersion != null && isUsingMinecraftProvidedJava);

        javaPathRow.setVisible(instance.javaVersion == null || !isUsingMinecraftProvidedJava);

        // Java Paramaters

        JPanel javaParametersPanel = new JPanel(new BorderLayout(8, 0));
        javaParametersPanel.setOpaque(false);
        JScrollPane javaParametersScrollPane = new JScrollPane(ScrollPaneConstants.VERTICAL_SCROLLBAR_AS_NEEDED,
            ScrollPaneConstants.HORIZONTAL_SCROLLBAR_NEVER);
        javaParametersScrollPane.setBorder(new FlatScrollPaneBorder());

        javaParameters = new JTextArea(3, 1);
        ((AbstractDocument) javaParameters.getDocument()).setDocumentFilter(
            new DocumentFilter() {
                @Override
                public void insertString(FilterBypass fb, int offset, String string, AttributeSet attr)
                    throws BadLocationException {
                    fb.insertString(offset, string.replaceAll("[\n\r]", ""), attr);
                }

                @Override
                public void replace(FilterBypass fb, int offset, int length, String text, AttributeSet attrs)
                    throws BadLocationException {
                    fb.replace(offset, length, text.replaceAll("[\n\r]", ""), attrs);
                }
            });
        javaParameters.setText(getIfNotNull(this.instance.launcher.javaArguments, App.settings.javaParameters));
        javaParameters.setLineWrap(true);
        javaParameters.setWrapStyleWord(true);
        JButton javaParametersResetButton = ModernDialogs.secondary(GetText.tr("Reset")).compact();
        javaParametersResetButton.addActionListener(e -> javaParameters.setText(App.settings.javaParameters));

        javaParametersScrollPane.setViewportView(javaParameters);
        javaParametersPanel.add(javaParametersScrollPane, BorderLayout.CENTER);

        JPanel paramsResetBox = new JPanel(new BorderLayout());
        paramsResetBox.setOpaque(false);
        paramsResetBox.add(javaParametersResetButton, BorderLayout.NORTH);

        javaParametersPanel.add(paramsResetBox, BorderLayout.EAST);

        JPanel javaParametersRow = SettingRows.stacked(GetText.tr("Java arguments"),
            GetText.tr("Extra options passed to Java. Memory is set above, so leave out -Xmx"),
            javaParametersPanel);

        // Runtime Override

        javaRuntimeOverride = new JComboBox<>();
        if (instance.javaVersion != null) {
            javaRuntimeOverride.addItem(new ComboItem<>(null, GetText.tr("Use default (recommended)")));
        } else {
            javaRuntimeOverride.addItem(new ComboItem<>(null, GetText.tr("Use system Java")));
        }

        int selectedIndexRuntime = 0;
        Map<String, List<JavaRuntime>> runtimes = Data.JAVA_RUNTIMES.getForSystem();
        for (String runtime : runtimes.keySet()) {
            List<JavaRuntime> runtimeObject = runtimes.get(runtime);

            if (runtimeObject != null && !runtimeObject.isEmpty()) {
                javaRuntimeOverride.addItem(
                    new ComboItem<>(runtime,
                        String.format("%s (Java %s)", runtime, runtimeObject.get(0).version.name)));

                if (this.instance.launcher.javaRuntimeOverride != null
                    && this.instance.launcher.javaRuntimeOverride.equals(runtime)) {
                    selectedIndexRuntime = javaRuntimeOverride.getItemCount() - 1;
                }
            }
        }

        javaRuntimeOverride.setSelectedIndex(selectedIndexRuntime);
        JPanel javaRuntimeOverrideRow = SettingRows.row(GetText.tr("Java runtime"),
            GetText.tr("Which of Mojang's Java runtimes starts this instance. Changing it is rarely needed"),
            SettingRows.dropdown(javaRuntimeOverride));
        javaRuntimeOverrideRow.setVisible(isUsingMinecraftProvidedJava);

        // Use Java Provided By Minecraft
        useJavaProvidedByMinecraft = SettingRows.triState(instance.launcher.useJavaProvidedByMinecraft,
            App.settings.useJavaProvidedByMinecraft);

        useJavaProvidedByMinecraft.addItemListener(e -> {
            if (e.getStateChange() == ItemEvent.SELECTED) {
                SwingUtilities.invokeLater(() -> {
                    if (useJavaProvidedByMinecraft.getSelectedIndex() == 2) {
                        int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Warning"))
                            .setType(DialogManager.WARNING)
                            .setContent(GetText.tr(
                                "Unchecking this is not recommended and may cause Minecraft to no longer run. Are you sure you want to do this?"))
                            .show();

                        if (ret != 0) {
                            useJavaProvidedByMinecraft.setSelectedIndex(0);
                        } else {
                            javaMinecraftProvidedRow.setVisible(false);
                            javaRuntimeOverrideRow.setVisible(false);

                            javaPathRow.setVisible(true);
                        }
                    } else if (useJavaProvidedByMinecraft.getSelectedIndex() == 1) {
                        javaMinecraftProvidedRow.setVisible(true);
                        javaRuntimeOverrideRow.setVisible(true);

                        javaPathRow.setVisible(false);
                    } else {
                        javaMinecraftProvidedRow.setVisible(App.settings.useJavaProvidedByMinecraft);
                        javaRuntimeOverrideRow.setVisible(App.settings.useJavaProvidedByMinecraft);

                        javaPathRow.setVisible(!App.settings.useJavaProvidedByMinecraft);
                    }
                    revalidate();
                    repaint();
                });
            }
        });

        JPanel useJavaProvidedByMinecraftRow = SettingRows.row(GetText.tr("Use Java provided by Minecraft"),
            GetText.tr("Recommended. Turning this off may stop Minecraft from starting"),
            useJavaProvidedByMinecraft);
        useJavaProvidedByMinecraftRow.setVisible(instance.javaVersion != null);

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Java"), useJavaProvidedByMinecraftRow,
            javaRuntimeOverrideRow, javaMinecraftProvidedRow, javaPathRow, javaParametersRow));

        // Disable Legacy Launching (hidden for legacy fabric as it never uses legacy launching)
        JPanel disableLegacyLaunchingRow = null;
        if (instance.launcher.loaderVersion == null || !instance.launcher.loaderVersion.isLegacyFabric()) {
            disableLegacyLaunching = SettingRows.triState(instance.launcher.disableLegacyLaunching,
                App.settings.disableLegacyLaunching);
            disableLegacyLaunchingRow = SettingRows.row(GetText.tr("Turn off legacy launching"),
                GetText.tr("For Minecraft older than 1.6. Only change this if an old version won't start"),
                disableLegacyLaunching);
        }

        // Use System GLFW
        useSystemGlfw = SettingRows.triState(instance.launcher.useSystemGlfw, App.settings.useSystemGlfw);

        // Use System OpenAL
        useSystemOpenAl = SettingRows.triState(instance.launcher.useSystemOpenAl, App.settings.useSystemOpenAl);

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Advanced"), disableLegacyLaunchingRow,
            SettingRows.row(GetText.tr("Use system GLFW"),
                GetText.tr("Use the GLFW library installed on this computer instead of Minecraft's own"),
                useSystemGlfw),
            SettingRows.row(GetText.tr("Use system OpenAL"),
                GetText.tr("Use the OpenAL sound library installed on this computer instead of Minecraft's own"),
                useSystemOpenAl)));
    }

    /**
     * PermGen (Metaspace on newer Java) only matters for Java 7 and older and for old modpacks that ask for a
     * minimum. It's also shown when it was changed for this instance, so it can be changed back.
     */
    private boolean needsPermGen() {
        if (instance.launcher.permGen != null
            || (instance.launcher.requiredPermGen != null && instance.launcher.requiredPermGen > 0)) {
            return true;
        }
        if (instance.javaVersion != null && instance.javaVersion.majorVersion > 0
            && instance.javaVersion.majorVersion < 8) {
            return true;
        }
        try {
            String path = getIfNotNull(instance.launcher.javaPath, App.settings.javaPath);
            if (path != null && !path.isEmpty()) {
                int major = Java.parseJavaVersionNumber(Java.versionFromReleaseFile(new File(path).toPath()));
                return major > 0 && major < 8;
            }
        } catch (Exception ignored) {
            // an unreadable Java folder just means it isn't an old Java we know about
        }
        return false;
    }

    private Integer getIfNotNull(Integer value, Integer defaultValue) {
        if (value != null) {
            return value;
        }

        return defaultValue;
    }

    private String getIfNotNull(String value, String defaultValue) {
        if (value != null) {
            return value;
        }

        return defaultValue;
    }

    public boolean isValidJavaPath() {
        File jPath = new File(javaPath.getText(), "bin");
        if (!jPath.exists()) {
            DialogManager.okDialog().setTitle(GetText.tr("Help")).setContent(new HTMLBuilder().center().text(GetText.tr(
                    "The Java Path you set is incorrect.<br/><br/>Please verify it points to the folder where the bin folder is and try again."))
                .build()).setType(DialogManager.ERROR).show();
            return false;
        }
        return true;
    }

    public boolean isValidJavaParamaters() {
        if (javaParameters.getText().contains("-Xmx")
            || javaParameters.getText().contains("-XX:PermSize")
            || javaParameters.getText().contains("-XX:MetaspaceSize")) {
            DialogManager.okDialog().setTitle(GetText.tr("Help")).setContent(new HTMLBuilder().center().text(GetText.tr(
                    "The entered Java Parameters were incorrect.<br/><br/>Please remove any references to Xmx or XX:PermSize."))
                .build()).setType(DialogManager.ERROR).show();
            return false;
        }
        return true;
    }

    public void saveSettings() {
        Integer maximumMemory = (Integer) this.maximumMemory.getValue();
        Integer permGen = (Integer) this.permGen.getValue();
        String javaPath = this.javaPath.getText();
        String javaParameters = this.javaParameters.getText();
        String javaRuntimeOverrideVal = ((ComboItem<String>) javaRuntimeOverride.getSelectedItem())
            .getValue();
        Boolean useJavaProvidedByMinecraftVal = ((ComboItem<Boolean>) useJavaProvidedByMinecraft.getSelectedItem())
            .getValue();
        Boolean disableLegacyLaunchingVal = disableLegacyLaunching == null ? null
            : ((ComboItem<Boolean>) disableLegacyLaunching.getSelectedItem()).getValue();
        Boolean useSystemGlfwVal = ((ComboItem<Boolean>) useSystemGlfw.getSelectedItem()).getValue();
        Boolean useSystemOpenAlVal = ((ComboItem<Boolean>) useSystemOpenAl.getSelectedItem()).getValue();

        this.instance.launcher.maximumMemory = (maximumMemory == App.settings.maximumMemory ? null : maximumMemory);
        this.instance.launcher.permGen = (permGen == App.settings.metaspace ? null : permGen);

        boolean instanceWillUseMinecraftProvidedJava = Optional.ofNullable(useJavaProvidedByMinecraftVal)
            .orElse(App.settings.useJavaProvidedByMinecraft);

        if (!instanceWillUseMinecraftProvidedJava || instance.javaVersion == null) {
            this.instance.launcher.javaPath = (javaPath.equals(App.settings.javaPath) ? null : javaPath);
        }

        this.instance.launcher.javaArguments = (javaParameters.equals(App.settings.javaParameters) ? null
            : javaParameters);

        this.instance.launcher.useJavaProvidedByMinecraft = useJavaProvidedByMinecraftVal;
        this.instance.launcher.disableLegacyLaunching = disableLegacyLaunchingVal;
        this.instance.launcher.javaRuntimeOverride = javaRuntimeOverrideVal;
        this.instance.launcher.useSystemGlfw = useSystemGlfwVal;
        this.instance.launcher.useSystemOpenAl = useSystemOpenAlVal;
    }
}
