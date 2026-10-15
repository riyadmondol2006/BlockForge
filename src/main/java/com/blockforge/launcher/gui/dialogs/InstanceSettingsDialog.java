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
import java.awt.CardLayout;
import java.awt.FlowLayout;
import java.awt.Rectangle;
import java.awt.Dimension;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;

import javax.swing.BorderFactory;
import javax.swing.JButton;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JPanel;
import javax.swing.Scrollable;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.gui.dialogs.instancesettings.CommandsInstanceSettingsTab;
import com.blockforge.launcher.gui.dialogs.instancesettings.GeneralInstanceSettingsTab;
import com.blockforge.launcher.gui.dialogs.instancesettings.JavaInstanceSettingsTab;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.TabStrip;

public class InstanceSettingsDialog extends JDialog {
    private final Instance instance;

    private final JPanel cards = new JPanel(new CardLayout());
    private TabStrip tabs;

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
        setSize(700, 660);
        setMinimumSize(new Dimension(640, 520));
        setLocationRelativeTo(App.launcher.getParent());
        setResizable(true);
        setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);

        // Tabs

        cards.setOpaque(false);
        cards.add(page(generalInstanceSettingsTab), "0");
        cards.add(page(javaInstanceSettingsTab), "1");
        cards.add(page(commandsInstanceSettingsTab), "2");

        tabs = new TabStrip(
                new String[] { GetText.tr("General"), GetText.tr("Java and Minecraft"), GetText.tr("Commands") },
                new Icons[] { Icons.PERSON, Icons.CUBE, Icons.TERMINAL },
                index -> ((CardLayout) cards.getLayout()).show(cards, String.valueOf(index)));

        JPanel tabRow = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        tabRow.setOpaque(false);
        tabRow.setBorder(BorderFactory.createEmptyBorder(0, 0, 12, 0));
        tabRow.add(tabs);

        JPanel body = new JPanel(new BorderLayout());
        body.setOpaque(false);
        body.add(tabRow, BorderLayout.NORTH);
        body.add(cards, BorderLayout.CENTER);

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
                body, ModernDialogs.footer(cancelButton, saveButton));
    }

    /** Shows one of the tabs: 0 general, 1 Java and Minecraft, 2 commands. */
    public void selectTab(int index) {
        tabs.select(index);
    }

    /** Puts a settings tab at the top of a page that scrolls up and down and always fits the window's width. */
    private static JComponent page(JPanel tab) {
        tab.setOpaque(false);
        tab.setBorder(BorderFactory.createEmptyBorder(4, 0, 12, 2));

        FitWidth holder = new FitWidth();
        holder.add(tab, BorderLayout.NORTH);

        return ModernDialogs.scroll(holder);
    }

    /** A see-through panel that is always as wide as the scroll pane it's in, so nothing is cut off on the right. */
    private static final class FitWidth extends JPanel implements Scrollable {
        private static final long serialVersionUID = 1L;

        FitWidth() {
            super(new BorderLayout());
            setOpaque(false);
        }

        @Override
        public Dimension getPreferredScrollableViewportSize() {
            return getPreferredSize();
        }

        @Override
        public int getScrollableUnitIncrement(Rectangle visibleRect, int orientation, int direction) {
            return 24;
        }

        @Override
        public int getScrollableBlockIncrement(Rectangle visibleRect, int orientation, int direction) {
            return Math.max(24, visibleRect.height - 48);
        }

        @Override
        public boolean getScrollableTracksViewportWidth() {
            return true;
        }

        @Override
        public boolean getScrollableTracksViewportHeight() {
            return false;
        }
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
