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

import java.awt.BorderLayout;
import java.awt.Component;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GridBagLayout;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JSlider;
import javax.swing.JSpinner;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.gui.modern.RoundPanel;
import com.blockforge.launcher.utils.ComboItem;

/**
 * The rows the instance settings tabs are made of, in the same style as the launcher's Settings page: a bold label
 * with a dim hint under it on the left and the control on the right, grouped in rounded cards.
 */
public final class SettingRows {
    /** width of dropdowns and other controls on the right of a row */
    public static final int CONTROL_WIDTH = 230;

    private SettingRows() {
    }

    /** Lays a tab out as a column of cards. */
    static void column(JPanel tab) {
        tab.setLayout(new BoxLayout(tab, BoxLayout.Y_AXIS));
        tab.setOpaque(false);
    }

    /** Adds a card to a tab with room under it. */
    static void addSection(JPanel tab, JComponent section) {
        if (tab.getComponentCount() > 0) {
            tab.add(spacer(14));
        }
        tab.add(section);
    }

    private static Component spacer(int height) {
        Component strut = Box.createVerticalStrut(height);
        ((JComponent) strut).setAlignmentX(Component.LEFT_ALIGNMENT);
        return strut;
    }

    /** A rounded card with a small accent heading and rows under it. */
    public static RoundPanel section(String heading, JComponent... rows) {
        RoundPanel card = new RoundPanel(new BorderLayout(), 20, Palette::card);
        card.line(Palette::outline);
        card.setAlignmentX(Component.LEFT_ALIGNMENT);
        card.pad(14, 18, 6, 18);

        JPanel list = new JPanel();
        list.setOpaque(false);
        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        JLabel h = Lbl.text(heading.toUpperCase(java.util.Locale.ROOT), 11f, true, Palette::accent);
        h.setAlignmentX(Component.LEFT_ALIGNMENT);
        h.setBorder(BorderFactory.createEmptyBorder(0, 0, 4, 0));
        list.add(h);
        for (JComponent row : rows) {
            if (row != null) {
                row.setAlignmentX(Component.LEFT_ALIGNMENT);
                list.add(row);
            }
        }
        card.add(list, BorderLayout.CENTER);
        return card;
    }

    /** Label and hint on the left, the control on the right. */
    public static JPanel row(String label, String hint, JComponent control) {
        JPanel row = new JPanel(new BorderLayout(16, 0));
        row.setOpaque(false);
        row.setBorder(BorderFactory.createEmptyBorder(9, 0, 9, 0));
        row.add(text(label, hint), BorderLayout.CENTER);

        JPanel holder = new JPanel(new GridBagLayout());
        holder.setOpaque(false);
        holder.add(control);
        row.add(holder, BorderLayout.EAST);
        return row;
    }

    /** Label and hint with a wide control under them, for paths and arguments. */
    public static JPanel stacked(String label, String hint, JComponent control) {
        JPanel row = new JPanel(new BorderLayout(0, 8));
        row.setOpaque(false);
        row.setBorder(BorderFactory.createEmptyBorder(9, 0, 9, 0));
        row.add(text(label, hint), BorderLayout.NORTH);
        row.add(control, BorderLayout.CENTER);
        return row;
    }

    private static JPanel text(String label, String hint) {
        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel l = Lbl.text(label, 14f, true, Palette::text);
        l.setAlignmentX(Component.LEFT_ALIGNMENT);
        text.add(l);
        if (hint != null) {
            text.add(Box.createVerticalStrut(2));
            Lbl.Paragraph h = new Lbl.Paragraph(hint, 12f, 3, 320, Palette::textDim).fillWidth();
            h.setAlignmentX(Component.LEFT_ALIGNMENT);
            text.add(h);
        }
        return text;
    }

    /** Gives a dropdown the standard width for the right of a row. */
    public static <T extends JComboBox<?>> T dropdown(T box) {
        ModernDialogs.preferWidth(box, CONTROL_WIDTH);
        return box;
    }

    /**
     * A dropdown for a setting that is either the launcher's choice (null), yes or no, in that order. The first item
     * says what the launcher's choice currently is.
     */
    public static JComboBox<ComboItem<Boolean>> triState(Boolean value, boolean launcherDefault) {
        JComboBox<ComboItem<Boolean>> box = new JComboBox<>();
        box.addItem(new ComboItem<>(null, launcherDefault ? GetText.tr("Launcher default (on)")
            : GetText.tr("Launcher default (off)")));
        box.addItem(new ComboItem<>(true, GetText.tr("On")));
        box.addItem(new ComboItem<>(false, GetText.tr("Off")));
        box.setSelectedIndex(value == null ? 0 : (value ? 1 : 2));
        return dropdown(box);
    }

    /** A number field with a unit after it. */
    public static JPanel withUnit(JSpinner spinner, String unit) {
        ((JSpinner.DefaultEditor) spinner.getEditor()).getTextField().setColumns(5);
        JPanel panel = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
        panel.setOpaque(false);
        panel.add(spinner);
        panel.add(Lbl.text(unit, 13f, false, Palette::textDim));
        return panel;
    }

    /**
     * A slider next to a memory spinner, kept in step with it. The spinner stays the value that is saved, so a value
     * the slider can't show exactly (or one outside its range) is never changed unless the slider is moved.
     */
    public static JPanel memory(JSpinner spinner, int min, int max) {
        int current = ((Number) spinner.getValue()).intValue();
        JSlider slider = new JSlider(min, Math.max(min + 256, max), Math.max(min, Math.min(max, current)));
        slider.setOpaque(false);
        slider.setPreferredSize(new Dimension(150, 32));
        boolean[] syncing = { false };
        slider.addChangeListener(e -> {
            if (syncing[0]) {
                return;
            }
            syncing[0] = true;
            spinner.setValue(Math.max(min, (slider.getValue() / 256) * 256));
            syncing[0] = false;
        });
        spinner.addChangeListener(e -> {
            if (syncing[0]) {
                return;
            }
            syncing[0] = true;
            slider.setValue(((Number) spinner.getValue()).intValue());
            syncing[0] = false;
        });

        JPanel panel = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
        panel.setOpaque(false);
        panel.add(slider);
        panel.add(withUnit(spinner, GetText.tr("MB")));
        return panel;
    }

    /** "8 GB" or "7.5 GB" */
    public static String gigabytes(int mb) {
        return mb % 1024 == 0 ? (mb / 1024) + " GB" : String.format("%.1f GB", mb / 1024f);
    }
}
