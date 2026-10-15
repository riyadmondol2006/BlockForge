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

import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.awt.Insets;
import java.awt.event.ItemEvent;

import javax.swing.JComboBox;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JTextField;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.utils.ComboItem;

public class CommandsInstanceSettingsTab extends JPanel {
    private final Instance instance;

    private final JComboBox<ComboItem<Boolean>> enableCommands;
    private final JTextField preLaunchCommand;
    private final JTextField postExitCommand;
    private final JTextField wrapperCommand;

    public CommandsInstanceSettingsTab(Instance instance) {
        this.instance = instance;

        SettingRows.column(this);

        // Enable commands

        enableCommands = SettingRows.triState(instance.launcher.enableCommands, App.settings.enableCommands);

        // Pre-launch command
        preLaunchCommand = new JTextField(App.settings.preLaunchCommand, 32);

        if (this.instance.launcher.preLaunchCommand != null) {
            preLaunchCommand.setText(this.instance.launcher.preLaunchCommand);
        }

        // Post-exit command
        postExitCommand = new JTextField(App.settings.postExitCommand, 32);

        if (this.instance.launcher.postExitCommand != null) {
            postExitCommand.setText(this.instance.launcher.postExitCommand);
        }

        // wrapper command
        wrapperCommand = new JTextField(App.settings.wrapperCommand, 32);

        if (this.instance.launcher.wrapperCommand != null) {
            wrapperCommand.setText(this.instance.launcher.wrapperCommand);
        }

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Commands"),
                SettingRows.row(GetText.tr("Run commands"),
                        GetText.tr("Turn the commands below on or off for this instance"), enableCommands),
                SettingRows.stacked(GetText.tr("Before launch"),
                        GetText.tr("Runs before the game starts. The game waits until it has finished"),
                        preLaunchCommand),
                SettingRows.stacked(GetText.tr("After exit"),
                        GetText.tr("Runs after the game closes, even if it crashed or was stopped"),
                        postExitCommand),
                SettingRows.stacked(GetText.tr("Wrapper command"),
                        GetText.tr("Starts the game through another program, such as prime-run on Linux. Use %command% for the launch command, or %\"command\"% for it as one quoted string"),
                        wrapperCommand)));

        // Parameter Information
        JPanel variables = new JPanel(new GridBagLayout());
        variables.setOpaque(false);
        String[][] rows = {
                { "$INST_NAME", GetText.tr("The name of the instance") },
                { "$INST_ID", GetText.tr("The name of the instance's root directory") },
                { "$INST_DIR", GetText.tr("The absolute path to the instance directory") },
                { "$INST_MC_DIR", GetText.tr("Alias for") + " $INST_DIR" },
                { "$INST_JAVA", GetText.tr("The absolute path to the java executable used for launch") },
                { "$INST_JAVA_ARGS", GetText.tr("The JVM parameters used for launch") } };
        GridBagConstraints gbc = new GridBagConstraints();
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        gbc.insets = new Insets(3, 0, 3, 16);
        gbc.gridy = 0;
        for (String[] row : rows) {
            gbc.gridx = 0;
            gbc.weightx = 0;
            JLabel name = Lbl.text(row[0], 12f, true, Palette::text);
            variables.add(name, gbc);
            gbc.gridx = 1;
            gbc.weightx = 1;
            variables.add(Lbl.text(row[1], 12f, false, Palette::textDim), gbc);
            gbc.gridy++;
        }

        SettingRows.addSection(this, SettingRows.section(GetText.tr("Variables"),
                SettingRows.stacked(GetText.tr("Available in every command"),
                        GetText.tr("Commands run in the folder of the instance that is launched or exited"),
                        variables)));

        enableCommands.addItemListener(e -> {
            if (e.getStateChange() == ItemEvent.SELECTED) {
                Boolean enabled = ((ComboItem<Boolean>) e.getItem()).getValue();

                toggleFields(enabled);
            }
        });

        toggleFields(instance.launcher.enableCommands);
    }

    private void toggleFields(Boolean enabled) {
        boolean fieldsEnabled = enabled != null && enabled;

        preLaunchCommand.setEnabled(fieldsEnabled);
        postExitCommand.setEnabled(fieldsEnabled);
        wrapperCommand.setEnabled(fieldsEnabled);
    }

    private String nullIfEmpty(String str) {
        if (str.isEmpty())
            return null;
        else
            return str;
    }

    public void saveSettings() {
        this.instance.launcher.enableCommands = ((ComboItem<Boolean>) enableCommands.getSelectedItem()).getValue();
        this.instance.launcher.preLaunchCommand = nullIfEmpty(preLaunchCommand.getText());
        this.instance.launcher.postExitCommand = nullIfEmpty(postExitCommand.getText());
        this.instance.launcher.wrapperCommand = nullIfEmpty(wrapperCommand.getText());
    }

}
