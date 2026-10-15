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
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.awt.Window;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;

import javax.swing.JButton;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JTextField;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.utils.Utils;

public class RenameInstanceDialog extends JDialog {

    private JTextField instanceName;

    private final Instance instance;

    public RenameInstanceDialog(Instance instance) {
        this(instance, App.launcher.getParent());
    }

    public RenameInstanceDialog(Instance instance, Window parent) {
        super(parent, GetText.tr("Renaming Instance"), ModalityType.DOCUMENT_MODAL);

        this.instance = instance;

        Analytics.sendScreenView("Rename Instance Dialog");

        setSize(420, 230);
        setLocationRelativeTo(parent);
        setIconImage(Utils.getImage("/assets/image/icon.png"));
        setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);
        setResizable(false);

        setupComponents();

        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosing(WindowEvent arg0) {
                close();
            }
        });
    }

    private void setupComponents() {
        // Top Panel Stuff
        JPanel top = ModernDialogs.header(GetText.tr("Renaming Instance"), this.instance.launcher.name, Icons.EDIT);

        // Middle Panel Stuff
        JPanel middle = new JPanel();
        middle.setOpaque(false);
        middle.setLayout(new GridBagLayout());
        GridBagConstraints gbc = new GridBagConstraints();

        gbc.gridx = 0;
        gbc.gridy = 0;
        gbc.insets = ModernDialogs.LABEL_INSETS;
        gbc.anchor = GridBagConstraints.BASELINE_TRAILING;
        JLabel instanceNameLabel = ModernDialogs.label(GetText.tr("Instance Name") + ": ");
        middle.add(instanceNameLabel, gbc);

        gbc.gridx++;
        gbc.insets = ModernDialogs.FIELD_INSETS;
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        gbc.weightx = 1;
        gbc.fill = GridBagConstraints.HORIZONTAL;
        instanceName = new JTextField(16);
        instanceName.setText(this.instance.launcher.name);
        middle.add(instanceName, gbc);

        // Bottom Panel Stuff
        JButton saveButton = ModernDialogs.primary(GetText.tr("Save"));
        saveButton.addActionListener(e -> {
            if (InstanceManager.isInstance(instanceName.getText())) {
                DialogManager.okDialog().setParent(RenameInstanceDialog.this).setTitle(GetText.tr("Error"))
                        .setContent(
                                GetText.tr("There is already an instance called {0}.<br/><br/>Rename it and try again.",
                                        instanceName.getText()))
                        .setType(DialogManager.ERROR).show();
            } else if (instanceName.getText().replaceAll("[^A-Za-z0-9]", "").isEmpty()) {
                DialogManager.okDialog().setTitle(GetText.tr("Error"))
                        .setContent(
                                new HTMLBuilder().center()
                                        .text(GetText.tr("Error") + "<br/><br/>" + GetText.tr(
                                                "The name {0} is invalid. It must contain at least 1 letter or number.",
                                                instanceName.getText()))
                                        .build())
                        .setType(DialogManager.ERROR).show();
            } else {
                if (!instance.rename(instanceName.getText())) {
                    LogManager.error("Unknown Error Occurred While Renaming Instance!");
                    DialogManager.okDialog().setParent(RenameInstanceDialog.this).setTitle(GetText.tr("Error"))
                            .setContent(new HTMLBuilder().center().text(GetText.tr(
                                    "An error occurred renaming the instance.<br/><br/>Please check the console and try again."))
                                    .build())
                            .setType(DialogManager.ERROR).show();
                }
                close();
            }
        });

        JPanel body = new JPanel(new BorderLayout());
        body.setOpaque(false);
        body.add(middle, BorderLayout.NORTH);

        ModernDialogs.install(this, top, body, ModernDialogs.footer(saveButton));
        ModernDialogs.growToFitOnShow(this);
    }

    private void close() {
        setVisible(false);
        dispose();
    }
}
