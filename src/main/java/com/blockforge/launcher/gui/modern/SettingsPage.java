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
package com.blockforge.launcher.gui.modern;

import java.awt.BorderLayout;
import java.awt.Cursor;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.GridBagLayout;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.RoundRectangle2D;
import java.util.List;
import java.util.function.Consumer;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JSlider;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.data.LauncherTheme;
import com.blockforge.launcher.evnt.manager.SettingsManager;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.viewmodel.impl.settings.GeneralSettingsViewModel;

/**
 * The settings people actually change, in plain language. Everything else is under "All settings".
 */
public class SettingsPage extends Page {
    private static final long serialVersionUID = 1L;

    private final GeneralSettingsViewModel viewModel = new GeneralSettingsViewModel();

    public SettingsPage() {
        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        column.setBorder(BorderFactory.createEmptyBorder(28, 36, 36, 36));

        JLabel title = Lbl.title(GetText.tr("Settings"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        JLabel sub = Lbl.dim(GetText.tr("Changes are saved as you make them."));
        sub.setAlignmentX(LEFT_ALIGNMENT);
        column.add(title);
        column.add(sub);
        column.add(Box.createVerticalStrut(22));

        column.add(section(GetText.tr("Appearance"), themeRow(), languageRow()));
        column.add(Box.createVerticalStrut(16));
        column.add(section(GetText.tr("Game"), memoryRow(),
            textRow(GetText.tr("Java path"), GetText.tr("Leave empty to use the Java that comes with Minecraft"),
                App.settings.javaPath, v -> App.settings.javaPath = v.isEmpty() ? null : v),
            textRow(GetText.tr("Java arguments"), GetText.tr("Extra options passed to the Java virtual machine"),
                App.settings.javaParameters, v -> App.settings.javaParameters = v),
            toggleRow(GetText.tr("Ask who's playing on start"),
                GetText.tr("Choose an offline name or sign in each time BlockForge opens"),
                App.settings.askPlayerOnStartup, v -> {
                    App.settings.askPlayerOnStartup = v;
                    SettingsManager.post();
                }),
            toggleRow(GetText.tr("Keep the launcher open"),
                GetText.tr("Leave BlockForge open while Minecraft is running"), App.settings.keepLauncherOpen,
                viewModel::setKeepLauncherOpen),
            toggleRow(GetText.tr("Show the console"), GetText.tr("Open the log window next to the launcher"),
                App.settings.enableConsole, viewModel::setEnableConsole)));
        column.add(Box.createVerticalStrut(16));
        column.add(section(GetText.tr("Privacy"),
            toggleRow(GetText.tr("Share anonymous usage statistics"),
                GetText.tr("Which screens are used and how often. Off unless you turn it on"),
                App.settings.enableAnalytics, v -> {
                    App.settings.enableAnalytics = v;
                    SettingsManager.post();
                }),
            toggleRow(GetText.tr("Report pack installs and play time"),
                GetText.tr("Counts for public modpacks, so authors can see how they are used. Off unless you turn it on"),
                App.settings.enableLogs, v -> {
                    App.settings.enableLogs = v;
                    SettingsManager.post();
                })));
        column.add(Box.createVerticalStrut(16));
        column.add(section(GetText.tr("Launcher"),
            toggleRow(GetText.tr("Tray icon"), GetText.tr("Show a tray menu while the launcher runs"),
                App.settings.enableTrayMenu, viewModel::setEnableTrayMenuOpen),
            buttonRow(GetText.tr("Data folder"), GetText.tr("Where instances, mods and logs are stored"),
                GetText.tr("Open"), Icons.FOLDER, () -> OS.openFileExplorer(FileSystem.BASE_DIR))));

        JPanel top = new JPanel(new BorderLayout());
        top.setOpaque(false);
        top.add(column, BorderLayout.NORTH);
        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(top, BorderLayout.CENTER);
        add(FitWidthPanel.scroll(fit), BorderLayout.CENTER);
    }

    private RoundPanel section(String heading, JComponent... rows) {
        RoundPanel card = new RoundPanel(new BorderLayout(), 24, Palette::card);
        card.line(Palette::outline);
        card.setAlignmentX(LEFT_ALIGNMENT);
        card.setMaximumSize(new Dimension(760, 1000));
        card.pad(16, 22, 8, 22);

        JPanel list = new JPanel();
        list.setOpaque(false);
        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        JLabel h = Lbl.text(heading.toUpperCase(java.util.Locale.ROOT), 11f, true, Palette::accent);
        h.setAlignmentX(LEFT_ALIGNMENT);
        h.setBorder(BorderFactory.createEmptyBorder(0, 0, 6, 0));
        list.add(h);
        for (JComponent row : rows) {
            row.setAlignmentX(LEFT_ALIGNMENT);
            list.add(row);
        }
        card.add(list, BorderLayout.CENTER);
        return card;
    }

    private JPanel row(String label, String hint, JComponent control) {
        JPanel row = new JPanel(new BorderLayout(16, 0));
        row.setOpaque(false);
        row.setBorder(BorderFactory.createEmptyBorder(8, 0, 8, 0));
        row.setMaximumSize(new Dimension(Integer.MAX_VALUE, 72));

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel l = Lbl.text(label, 14f, true, Palette::text);
        l.setAlignmentX(LEFT_ALIGNMENT);
        text.add(l);
        if (hint != null) {
            JLabel h = Lbl.text(hint, 12f, false, Palette::textDim);
            h.setAlignmentX(LEFT_ALIGNMENT);
            text.add(h);
        }
        row.add(text, BorderLayout.CENTER);

        JPanel holder = new JPanel(new GridBagLayout());
        holder.setOpaque(false);
        holder.add(control);
        row.add(holder, BorderLayout.EAST);
        return row;
    }

    private JPanel toggleRow(String label, String hint, boolean value, Consumer<Boolean> onChange) {
        return row(label, hint, new Toggle(value, onChange));
    }

    private JPanel buttonRow(String label, String hint, String button, Icons icon, Runnable action) {
        Btn b = new Btn(button, Btn.Kind.TONAL).icon(icon).compact();
        b.addActionListener(e -> action.run());
        return row(label, hint, b);
    }

    private JPanel textRow(String label, String hint, String value, Consumer<String> save) {
        SearchBox field = new SearchBox(null, false, 22);
        field.setText(value == null ? "" : value);
        field.setPreferredSize(new Dimension(300, 40));
        Runnable commit = () -> {
            save.accept(field.getText().trim());
            SettingsManager.post();
        };
        field.addActionListener(e -> commit.run());
        field.addFocusListener(new java.awt.event.FocusAdapter() {
            @Override
            public void focusLost(java.awt.event.FocusEvent e) {
                commit.run();
            }
        });
        return row(label, hint, field);
    }

    private JPanel themeRow() {
        List<LauncherTheme> themes = viewModel.getThemes();
        JComboBox<String> box = new JComboBox<>();
        int selected = 0;
        for (int i = 0; i < themes.size(); i++) {
            box.addItem(themes.get(i).label);
            if (themes.get(i).id.equals(App.settings.theme)) {
                selected = i;
            }
        }
        box.setSelectedIndex(selected);
        box.setPreferredSize(new Dimension(230, 38));
        box.addActionListener(e -> {
            int index = box.getSelectedIndex();
            if (index >= 0 && !themes.get(index).id.equals(App.settings.theme)) {
                viewModel.setSelectedTheme(themes.get(index).id);
                repaintAll();
            }
        });
        return row(GetText.tr("Theme"), GetText.tr("Light and dark looks for the whole launcher"), box);
    }

    private JPanel languageRow() {
        String[] languages = viewModel.getLanguages();
        JComboBox<String> box = new JComboBox<>(languages);
        for (int i = 0; i < languages.length; i++) {
            if (languages[i].equals(App.settings.language)) {
                box.setSelectedIndex(i);
            }
        }
        box.setPreferredSize(new Dimension(230, 38));
        box.addActionListener(e -> {
            String chosen = (String) box.getSelectedItem();
            if (chosen != null && !chosen.equals(App.settings.language)) {
                viewModel.setSelectedLanguage(chosen);
            }
        });
        return row(GetText.tr("Language"), null, box);
    }

    private JPanel memoryRow() {
        int max = Math.max(2048, OS.getSafeMaximumRam());
        int min = 1024;
        int current = Math.max(min, Math.min(max, App.settings.maximumMemory));

        JSlider slider = new JSlider(min, max, current);
        slider.setOpaque(false);
        slider.setPreferredSize(new Dimension(220, 32));
        slider.setMajorTickSpacing(1024);
        slider.setSnapToTicks(false);
        JLabel value = Lbl.text(memoryText(current), 13f, true, Palette::accent);
        value.setPreferredSize(new Dimension(64, 20));
        value.setHorizontalAlignment(JLabel.RIGHT);

        slider.addChangeListener(e -> {
            int v = (slider.getValue() / 256) * 256;
            value.setText(memoryText(v));
            if (!slider.getValueIsAdjusting()) {
                App.settings.maximumMemory = Math.max(min, v);
                SettingsManager.post();
            }
        });

        JPanel control = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        control.setOpaque(false);
        control.add(slider);
        control.add(value);
        return row(GetText.tr("Memory"), GetText.tr("How much RAM Minecraft may use"), control);
    }

    private static String memoryText(int mb) {
        return mb % 1024 == 0 ? (mb / 1024) + " GB" : String.format("%.1f GB", mb / 1024f);
    }

    private void repaintAll() {
        if (App.launcherFrame != null) {
            App.launcherFrame.repaint();
        }
    }

    /** A pill shaped on/off switch. */
    private static final class Toggle extends JComponent {
        private static final long serialVersionUID = 1L;

        private boolean on;
        private final Consumer<Boolean> onChange;

        Toggle(boolean on, Consumer<Boolean> onChange) {
            this.on = on;
            this.onChange = onChange;
            setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
            setFocusable(true);
            addMouseListener(new MouseAdapter() {
                @Override
                public void mouseClicked(MouseEvent e) {
                    flip();
                }
            });
            getInputMap(WHEN_FOCUSED).put(javax.swing.KeyStroke.getKeyStroke("SPACE"), "flip");
            getActionMap().put("flip", new javax.swing.AbstractAction() {
                private static final long serialVersionUID = 1L;

                @Override
                public void actionPerformed(java.awt.event.ActionEvent e) {
                    flip();
                }
            });
        }

        @Override
        public javax.accessibility.AccessibleContext getAccessibleContext() {
            if (accessibleContext == null) {
                accessibleContext = new AccessibleJComponent() {
                    @Override
                    public javax.accessibility.AccessibleRole getAccessibleRole() {
                        return javax.accessibility.AccessibleRole.TOGGLE_BUTTON;
                    }
                };
            }
            return accessibleContext;
        }

        private void flip() {
            on = !on;
            repaint();
            onChange.accept(on);
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(52, 30);
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            int w = getWidth();
            int h = getHeight();
            g.setColor(on ? Palette.accent() : Palette.field());
            g.fill(new RoundRectangle2D.Float(0, 0, w, h, h, h));
            if (!on) {
                g.setColor(Palette.outline());
                g.draw(new RoundRectangle2D.Float(0.5f, 0.5f, w - 1, h - 1, h, h));
            }
            int knob = h - 8;
            g.setColor(on ? Palette.onAccent() : Palette.textDim());
            g.fillOval(on ? w - knob - 4 : 4, 4, knob, knob);
            if (isFocusOwner() && FocusRing.visible()) {
                g.setColor(Palette.accent());
                g.setStroke(new java.awt.BasicStroke(2f));
                g.draw(new RoundRectangle2D.Float(-1, -1, w + 1, h + 1, h + 2, h + 2));
            }
            g.dispose();
        }
    }
}
