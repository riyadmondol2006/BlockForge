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
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;

import javax.swing.BorderFactory;
import javax.swing.JButton;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JPanel;
import javax.swing.JTabbedPane;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.gui.dialogs.instancesettings.CommandsInstanceSettingsTab;
import com.blockforge.launcher.gui.dialogs.instancesettings.GeneralInstanceSettingsTab;
import com.blockforge.launcher.gui.dialogs.instancesettings.JavaInstanceSettingsTab;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;

public class InstanceSettingsDialog extends JDialog {
    private final Instance instance;

    private final JTabbedPane tabbedPane = new JTabbedPane(JTabbedPane.TOP);

    private final GeneralInstanceSettingsTab generalInstanceSettingsTab;
    private final JavaInstanceSettingsTab javaInstanceSettingsTab;
    private final CommandsInstanceSettingsTab commandsInstanceSettingsTab;

    public InstanceSettingsDialog(Instance instance) {
        // #. {0} is the name of the instance
        super(App.launcher.getParent(), GetText.tr("{0} Settings", instance.launcher.name),
                ModalityType.DOCUMENT_MODAL);
        this.instance = instance;

        this.generalInstanceSettingsTab = new GeneralInstanceSettingsTab(instance);
        this.javaInstanceSettingsTab = new JavaInstanceSettingsTab(instance);
        this.commandsInstanceSettingsTab = new CommandsInstanceSettingsTab(instance);

        setupComponents();
        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosing(WindowEvent arg0) {
                close();
            }
        });
    }

    private void setupComponents() {
        setSize(820, 640);
        setMinimumSize(new Dimension(820, 640));
        setLocationRelativeTo(App.launcher.getParent());
        setResizable(false);
        setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);

        // Tabbed Pane

        tabbedPane.setFont(Palette.bold(14f));
        tabbedPane.addTab(GetText.tr("General"), page(generalInstanceSettingsTab));
        tabbedPane.addTab(GetText.tr("Java/Minecraft"), page(javaInstanceSettingsTab));
        tabbedPane.addTab(GetText.tr("Commands"), page(commandsInstanceSettingsTab));
        tabbedPane.setOpaque(false);

        // Bottom Panel

        JButton saveButton = ModernDialogs.primary(GetText.tr("Save"));
        saveButton.addActionListener(arg0 -> {
            if (javaInstanceSettingsTab.isValidJavaPath() && javaInstanceSettingsTab.isValidJavaParamaters()
                    && generalInstanceSettingsTab.isValidQuickPlayOptionValue()) {
                saveSettings();
                App.TOASTER.pop("Instance Settings Saved");
                close();
            }
        });

        JButton cancelButton = ModernDialogs.secondary(GetText.tr("Cancel"));
        cancelButton.addActionListener(arg0 -> close());

        ModernDialogs.install(this,
                ModernDialogs.header(GetText.tr("Settings"), instance.launcher.name, Icons.SETTINGS),
                tabbedPane, ModernDialogs.footer(cancelButton, saveButton));
    }

    /** Puts a settings tab at the top of a scrollable page with some room around it. */
    private static JComponent page(JPanel tab) {
        ModernDialogs.blend(tab);
        tab.setOpaque(false);
        tab.setBorder(BorderFactory.createEmptyBorder(18, 8, 18, 8));

        JPanel holder = new JPanel(new BorderLayout());
        holder.setOpaque(false);
        holder.add(tab, BorderLayout.NORTH);

        return ModernDialogs.scroll(holder);
    }

    private void close() {
        setVisible(false);
        dispose();
    }

    private void saveSettings() {
        generalInstanceSettingsTab.saveSettings();
        javaInstanceSettingsTab.saveSettings();
        commandsInstanceSettingsTab.saveSettings();

        this.instance.save();
    }

}
