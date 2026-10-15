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
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;

import javax.swing.JButton;
import javax.swing.JComboBox;
import javax.swing.JDialog;
import javax.swing.JFrame;
import javax.swing.JLabel;
import javax.swing.JPanel;

import com.blockforge.launcher.App;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.ModernDialogs;

public class FileTypeDialog extends JDialog {

    private final JComboBox<String> selector;

    private boolean closed = false;

    public FileTypeDialog(String title, String labelName, String bottomText, String selectorText, String[] subOptions) {
        super(App.launcher.getParent(), title, ModalityType.DOCUMENT_MODAL);
        setSize(440, 230);
        setLocationRelativeTo(null);
        setDefaultCloseOperation(JFrame.DO_NOTHING_ON_CLOSE);
        setResizable(false);

        // Top Panel Stuff
        JPanel top = ModernDialogs.header(labelName, null, Icons.FOLDER);

        // Middle Panel Stuff
        JPanel middle = new JPanel();
        middle.setOpaque(false);
        middle.setLayout(new GridBagLayout());
        GridBagConstraints gbc = new GridBagConstraints();

        gbc.gridx = 0;
        gbc.gridy = 0;
        gbc.insets = ModernDialogs.LABEL_INSETS;
        gbc.anchor = GridBagConstraints.BASELINE_TRAILING;
        JLabel selectorLabel = ModernDialogs.label(selectorText + ": ");
        middle.add(selectorLabel, gbc);

        gbc.gridx++;
        gbc.insets = ModernDialogs.FIELD_INSETS;
        gbc.weightx = 1;
        gbc.fill = GridBagConstraints.HORIZONTAL;
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        selector = new JComboBox<>();
        for (String item : subOptions) {
            selector.addItem(item);
        }
        middle.add(selector, gbc);

        // Bottom Panel Stuff
        JButton bottomButton = ModernDialogs.primary(bottomText);
        bottomButton.addActionListener(e -> close());

        JPanel body = new JPanel(new BorderLayout());
        body.setOpaque(false);
        body.add(middle, BorderLayout.NORTH);

        ModernDialogs.install(this, top, body, ModernDialogs.footer(bottomButton));
        ModernDialogs.growToFitOnShow(this);

        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosing(WindowEvent arg0) {
                closed = true;
                close();
            }
        });
    }

    private void close() {
        setVisible(false);
        dispose();
    }

    public boolean wasClosed() {
        return this.closed;
    }

    public String getSelectorValue() {
        return (String) this.selector.getSelectedItem();
    }

}
