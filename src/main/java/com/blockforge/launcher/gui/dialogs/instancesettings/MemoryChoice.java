/*
 * BlockForge - a fork of ATLauncher (https://github.com/ATLauncher/ATLauncher)
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

import java.awt.FlowLayout;

import javax.swing.JComboBox;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JSpinner;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.InstanceLauncher;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.mclauncher.InstanceMemory;
import com.blockforge.launcher.utils.ComboItem;

/**
 * An instance's memory: automatic (picked for this game and this computer, with the amount shown), the launcher's
 * setting, or a custom amount in the spinner next to it.
 */
final class MemoryChoice extends JPanel {
    private static final long serialVersionUID = 1L;

    static final String FOLLOW = "follow";
    static final String AUTOMATIC = "automatic";
    static final String CUSTOM = "custom";

    private final JComboBox<ComboItem<String>> choice = new JComboBox<>();
    private final JSpinner custom;
    private final JLabel hint = new JLabel(" ");
    private final Instance instance;
    private final boolean launcherAutomatic;

    /** @param custom the spinner for a custom amount, shown only when "Custom" is chosen */
    MemoryChoice(Instance instance, JSpinner custom) {
        super(new FlowLayout(FlowLayout.LEFT, 8, 0));
        this.instance = instance;
        this.custom = custom;
        this.launcherAutomatic = !Boolean.FALSE.equals(App.settings.automaticMemory);

        if (launcherAutomatic) {
            choice.addItem(new ComboItem<>(FOLLOW, GetText.tr("Automatic (recommended)")));
        } else {
            choice.addItem(new ComboItem<>(FOLLOW, GetText.tr("Launcher setting ({0} MB)", App.settings.maximumMemory)));
            choice.addItem(new ComboItem<>(AUTOMATIC, GetText.tr("Automatic (recommended)")));
        }
        choice.addItem(new ComboItem<>(CUSTOM, GetText.tr("Custom")));
        if (Boolean.TRUE.equals(instance.launcher.automaticMemory)) {
            choice.setSelectedIndex(launcherAutomatic ? 0 : 1);
        } else if (instance.launcher.maximumMemory != null) {
            choice.setSelectedIndex(choice.getItemCount() - 1);
        } else {
            choice.setSelectedIndex(0);
        }
        ModernDialogs.preferWidth(choice, 220);
        hint.setEnabled(false);

        add(choice);
        add(custom);
        add(hint);
        choice.addActionListener(e -> update());
        update();
    }

    @SuppressWarnings("unchecked")
    String selected() {
        Object item = choice.getSelectedItem();
        return item == null ? FOLLOW : ((ComboItem<String>) item).getValue();
    }

    private void update() {
        String chosen = selected();
        boolean automatic = AUTOMATIC.equals(chosen) || (FOLLOW.equals(chosen) && launcherAutomatic);
        custom.setVisible(CUSTOM.equals(chosen));
        if (automatic) {
            hint.setText(GetText.tr("Working it out..."));
            // reads the instance's folders, so not on the Swing thread
            App.TASKPOOL.execute(() -> {
                String text = InstanceMemory.describeAutomatic(instance);
                SwingUtilities.invokeLater(() -> {
                    if (chosen.equals(selected())) {
                        hint.setText(text);
                    }
                });
            });
        } else {
            hint.setText(CUSTOM.equals(chosen) ? GetText.tr("MB") : " ");
        }
        revalidate();
    }

    /** Writes the choice into the instance's settings. */
    void save(InstanceLauncher launcher) {
        String chosen = selected();
        if (CUSTOM.equals(chosen)) {
            launcher.automaticMemory = null;
            launcher.maximumMemory = (Integer) custom.getValue();
        } else if (AUTOMATIC.equals(chosen)) {
            launcher.automaticMemory = true;
            launcher.maximumMemory = null;
        } else {
            // follow the launcher setting (which may itself be automatic)
            launcher.automaticMemory = null;
            launcher.maximumMemory = null;
        }
    }
}
