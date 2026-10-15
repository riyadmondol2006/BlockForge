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
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.Insets;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;

import javax.swing.BorderFactory;
import javax.swing.JButton;
import javax.swing.JDialog;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JTextArea;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.utils.OS;

public class ModrinthExportOverridesDialog extends JDialog {
    public ModrinthExportOverridesDialog(Dialog parent, String text) {
        super(parent, GetText.tr("Overrides Included"), true);

        setMinimumSize(new Dimension(580, 460));
        setLocationRelativeTo(parent);

        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosing(WindowEvent arg0) {
                close();
            }
        });

        // Create the text area
        Lbl.Paragraph infoLabel = new Lbl.Paragraph(GetText.tr(
                "Your exported instance contains mods not on Modrinth and were included in the overrides folder. If you're uploading this to Modrinth, you will need to make sure you have permissions to distribute the below mods, else your modpack will get denied on Modrinth."),
                13f, 6, 520, Palette::textDim);
        infoLabel.setBorder(BorderFactory.createEmptyBorder(0, 0, 14, 0));

        // Create the text area
        JTextArea textArea = new JTextArea();
        textArea.setText(text);
        textArea.setMargin(new Insets(8, 10, 8, 10));

        JScrollPane scrollPane = new JScrollPane(textArea);
        scrollPane.setVerticalScrollBarPolicy(JScrollPane.VERTICAL_SCROLLBAR_ALWAYS);
        scrollPane.getVerticalScrollBar().setUnitIncrement(16);

        JPanel body = new JPanel(new BorderLayout());
        body.setOpaque(false);
        body.add(infoLabel, BorderLayout.NORTH);
        body.add(scrollPane, BorderLayout.CENTER);

        // Copy button
        JButton copyButton = ModernDialogs.secondary(GetText.tr("Copy"));
        copyButton.addActionListener(e -> OS.copyToClipboard(text));

        // Close button
        JButton closeButton = ModernDialogs.primary(GetText.tr("Close"));
        closeButton.addActionListener(e -> close());

        ModernDialogs.install(this, ModernDialogs.header(GetText.tr("Overrides Included"), null, Icons.INFO), body,
                ModernDialogs.footer(copyButton, closeButton));
    }

    private void close() {
        setVisible(false);
        dispose();
    }
}
