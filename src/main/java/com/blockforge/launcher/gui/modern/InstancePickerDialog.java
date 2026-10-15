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
import java.awt.Color;
import java.awt.Cursor;
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Window;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.image.BufferedImage;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComboBox;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.KeyStroke;
import javax.swing.WindowConstants;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.content.ContentService;
import com.blockforge.launcher.content.DroppedFile;
import com.blockforge.launcher.content.LoaderSupport;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.managers.InstanceManager;
import com.blockforge.launcher.utils.ComboItem;

/**
 * "Add 3 files to which instance?" Shown when files are dropped somewhere other than on an instance. Instances the
 * files suit come first; the others are marked with what might not work, but can still be picked.
 */
public final class InstancePickerDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    /** What the player picked. */
    public static final class Choice {
        public final Instance instance;
        /** true when they asked to make a new instance instead */
        public final boolean newInstance;

        Choice(Instance instance, boolean newInstance) {
            this.instance = instance;
            this.newInstance = newInstance;
        }
    }

    /** One instance in the list with what might not work there. */
    static final class Option {
        final Instance instance;
        final List<String> problems;

        Option(Instance instance, List<String> problems) {
            this.instance = instance;
            this.problems = problems;
        }
    }

    private final List<Row> rows = new ArrayList<>();
    private final Btn add = new Btn("", Btn.Kind.FILLED).icon(Icons.PLUS);
    private Row selected;
    private Choice choice;

    private InstancePickerDialog(Window owner, List<DroppedFile> files, List<Option> options, Option preselect) {
        super(owner, Constants.LAUNCHER_NAME, Dialog.ModalityType.DOCUMENT_MODAL);
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);

        JPanel root = new JPanel(new BorderLayout(0, 18)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(24, 26, 22, 26));

        JPanel head = new JPanel();
        head.setOpaque(false);
        head.setLayout(new BoxLayout(head, BoxLayout.Y_AXIS));
        JLabel title = Lbl.text(files.size() == 1 ? GetText.tr("Add {0} to which instance?", files.get(0).displayName())
            : GetText.tr("Add {0} files to which instance?", files.size()), 19f, true, Palette::text);
        JLabel sub = Lbl.text(FileDropFlow.describe(files), 13f, false, Palette::textDim);
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        head.add(title);
        head.add(Box.createVerticalStrut(4));
        head.add(sub);
        root.add(head, BorderLayout.NORTH);

        FitWidthPanel list = new FitWidthPanel(0);
        list.setLayout(new BoxLayout(list, BoxLayout.Y_AXIS));
        for (Option option : options) {
            Row row = new Row(option);
            rows.add(row);
            if (list.getComponentCount() > 0) {
                list.add(Box.createVerticalStrut(8));
            }
            list.add(row);
        }
        JPanel listWrap = new JPanel(new BorderLayout());
        listWrap.setOpaque(false);
        listWrap.add(list, BorderLayout.NORTH);
        FitWidthPanel scrollContent = new FitWidthPanel(0);
        scrollContent.setLayout(new BorderLayout());
        scrollContent.add(listWrap, BorderLayout.CENTER);
        root.add(FitWidthPanel.scroll(scrollContent), BorderLayout.CENTER);

        JPanel footer = new JPanel(new BorderLayout());
        footer.setOpaque(false);
        Btn create = new Btn(GetText.tr("New instance..."), Btn.Kind.TEXT).icon(Icons.PLUS);
        create.addActionListener(e -> {
            choice = new Choice(null, true);
            dispose();
        });
        footer.add(create, BorderLayout.WEST);
        JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        right.setOpaque(false);
        Btn cancel = new Btn(GetText.tr("Cancel"), Btn.Kind.TONAL);
        cancel.addActionListener(e -> dispose());
        add.addActionListener(e -> confirm());
        right.add(cancel);
        right.add(add);
        footer.add(right, BorderLayout.EAST);
        root.add(footer, BorderLayout.SOUTH);
        getRootPane().setDefaultButton(add);
        getRootPane().registerKeyboardAction(e -> dispose(), KeyStroke.getKeyStroke("ESCAPE"),
            JComponent.WHEN_IN_FOCUSED_WINDOW);

        for (Row row : rows) {
            if (row.option == preselect) {
                select(row);
            }
        }
        if (selected == null && !rows.isEmpty()) {
            select(rows.get(0));
        }
        add.setEnabled(selected != null);

        setContentPane(root);
        int height = Math.min(620, 210 + options.size() * 76);
        setPreferredSize(new Dimension(620, Math.max(360, height)));
        pack();
        setLocationRelativeTo(owner);
    }

    private void select(Row row) {
        selected = row;
        for (Row r : rows) {
            r.repaint();
        }
        add.setText(GetText.tr("Add to {0}", shorten(row.option.instance.launcher.name)));
        add.revalidate();
        add.setEnabled(true);
    }

    private static String shorten(String name) {
        return name.length() > 26 ? name.substring(0, 25) + "…" : name;
    }

    private void confirm() {
        if (selected != null) {
            choice = new Choice(selected.option.instance, false);
            dispose();
        }
    }

    /**
     * Asks which instance dropped files go into.
     *
     * @return the choice, or null if the player cancelled
     */
    public static Choice pick(Window owner, List<DroppedFile> files) {
        List<Option> options = options(InstanceManager.getInstances(), files);
        Option preselect = null;
        for (Option option : options) {
            if (option.problems.isEmpty()) {
                preselect = option;
                break;
            }
        }
        InstancePickerDialog dialog = new InstancePickerDialog(owner, files, options,
            preselect != null ? preselect : options.isEmpty() ? null : options.get(0));
        dialog.setVisible(true);
        return dialog.choice;
    }

    /** Instances with what might not work in each: the ones without problems first, then the most recently played. */
    static List<Option> options(List<Instance> instances, List<DroppedFile> files) {
        List<Option> options = new ArrayList<>();
        for (Instance instance : instances) {
            options.add(new Option(instance, problems(instance, files)));
        }
        options.sort(Comparator.comparing((Option o) -> !o.problems.isEmpty())
            .thenComparing(o -> lastPlayed(o.instance), Comparator.reverseOrder()));
        return options;
    }

    private static Instant lastPlayed(Instance instance) {
        try {
            return instance.getLastPlayedOrEpoch();
        } catch (RuntimeException e) {
            return Instant.EPOCH;
        }
    }

    static List<String> problems(Instance instance, List<DroppedFile> files) {
        String key = LoaderSupport.key(instance.launcher.loaderVersion);
        List<String> problems = new ArrayList<>();
        boolean dataPacks = false;
        for (DroppedFile file : files) {
            String problem = file.problemFor(key, instance.id);
            if (problem != null && !problems.contains(problem)) {
                problems.add(problem);
            }
            dataPacks |= file.kind == DroppedFile.Kind.DATA_PACK;
        }
        if (dataPacks && new ContentService(instance).worlds().isEmpty()) {
            problems.add(GetText.tr("No worlds yet for the data packs"));
        }
        return problems;
    }

    /** One instance to pick. */
    private final class Row extends RoundPanel {
        private static final long serialVersionUID = 1L;

        final Option option;

        Row(Option option) {
            super(new BorderLayout(14, 0), 18, Palette::card);
            this.option = option;
            hover(Palette::cardHover);
            pad(10, 12, 10, 14);
            setAlignmentX(LEFT_ALIGNMENT);
            setMaximumSize(new Dimension(Integer.MAX_VALUE, 68));
            setPreferredSize(new Dimension(400, 68));
            setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
            Instance instance = option.instance;

            ContentCards.Picture picture = new ContentCards.Picture(44, Icons.CUBE);
            java.io.File custom = instance.getRoot().resolve("instance.png").toFile();
            ImageCache.async(() -> {
                BufferedImage source = ImageCache.toBuffered(instance.getImage());
                return source == null ? null : ImageCache.cover(source, 88, 88);
            }, "instance-square:" + instance.getRoot() + "#" + custom.lastModified(), picture::setImage);
            add(picture, BorderLayout.WEST);

            JPanel text = new JPanel();
            text.setOpaque(false);
            text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
            JLabel name = Lbl.text(instance.launcher.name, 14f, true, Palette::text);
            String loader = LoaderSupport.key(instance.launcher.loaderVersion);
            JLabel meta = Lbl.text("Minecraft " + instance.id + "  ·  "
                + (loader == null ? GetText.tr("no mod loader") : LoaderSupport.displayName(loader)), 12f, false,
                Palette::textDim);
            name.setAlignmentX(LEFT_ALIGNMENT);
            meta.setAlignmentX(LEFT_ALIGNMENT);
            text.add(Box.createVerticalGlue());
            text.add(name);
            text.add(Box.createVerticalStrut(1));
            text.add(meta);
            if (!option.problems.isEmpty()) {
                String first = option.problems.get(0);
                if (option.problems.size() > 1) {
                    first += GetText.tr(" (and {0} more)", option.problems.size() - 1);
                }
                JLabel warning = Lbl.text(first, 12f, true, Palette::warn);
                warning.setIcon(Icons.WARNING.asIcon(14, Palette::warn));
                warning.setIconTextGap(5);
                warning.setToolTipText(String.join("\n", option.problems));
                warning.setAlignmentX(LEFT_ALIGNMENT);
                text.add(Box.createVerticalStrut(2));
                text.add(warning);
            }
            text.add(Box.createVerticalGlue());
            add(text, BorderLayout.CENTER);

            MouseAdapter click = new MouseAdapter() {
                @Override
                public void mousePressed(MouseEvent e) {
                    select(Row.this);
                }

                @Override
                public void mouseClicked(MouseEvent e) {
                    if (e.getClickCount() == 2) {
                        confirm();
                    }
                }
            };
            addMouseListener(click);
            text.addMouseListener(click);
            picture.addMouseListener(click);
        }

        @Override
        protected void paintComponent(java.awt.Graphics g0) {
            boolean on = selected == this;
            java.awt.Graphics2D g = (java.awt.Graphics2D) g0.create();
            g.setRenderingHint(java.awt.RenderingHints.KEY_ANTIALIASING, java.awt.RenderingHints.VALUE_ANTIALIAS_ON);
            java.awt.geom.RoundRectangle2D shape = new java.awt.geom.RoundRectangle2D.Float(0, 0, getWidth(),
                getHeight(), 18, 18);
            g.setColor(on ? Palette.accentSoft() : isHovered() ? Palette.cardHover() : Palette.card());
            g.fill(shape);
            g.setColor(on ? Palette.accent() : Palette.outline());
            g.setStroke(new java.awt.BasicStroke(on ? 2f : 1f));
            g.draw(new java.awt.geom.RoundRectangle2D.Float(on ? 1 : 0.5f, on ? 1 : 0.5f, getWidth() - (on ? 2 : 1),
                getHeight() - (on ? 2 : 1), 18, 18));
            g.dispose();
        }
    }

    // ---------------------------------------------------------------- worlds for data packs

    /**
     * Asks which world data packs go into.
     *
     * @return folder names of the chosen worlds, or null if the player cancelled
     */
    public static List<String> pickWorlds(Window owner, Instance instance, List<String> worlds, int packs) {
        JDialog dialog = new JDialog(owner, Constants.LAUNCHER_NAME, Dialog.ModalityType.DOCUMENT_MODAL);
        dialog.setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
        JPanel root = new JPanel(new BorderLayout(0, 18)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(24, 26, 22, 26));

        JPanel head = new JPanel();
        head.setOpaque(false);
        head.setLayout(new BoxLayout(head, BoxLayout.Y_AXIS));
        JLabel title = Lbl.text(packs == 1 ? GetText.tr("Which world gets the data pack?")
            : GetText.tr("Which world gets the {0} data packs?", packs), 18f, true, Palette::text);
        JLabel sub = Lbl.text(GetText.tr("Data packs belong to a world in {0}.", instance.launcher.name), 13f,
            false, Palette::textDim);
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        head.add(title);
        head.add(Box.createVerticalStrut(4));
        head.add(sub);
        root.add(head, BorderLayout.NORTH);

        JComboBox<ComboItem<String>> box = new JComboBox<>();
        if (worlds.size() > 1) {
            box.addItem(new ComboItem<>(null, GetText.tr("All worlds ({0})", worlds.size())));
        }
        for (String world : worlds) {
            box.addItem(new ComboItem<>(world, world));
        }
        SearchBox.styleLike(box, 420);
        JPanel middle = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        middle.setOpaque(false);
        middle.add(box);
        root.add(middle, BorderLayout.CENTER);

        final List<List<String>> answer = new ArrayList<>();
        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        buttons.setOpaque(false);
        Btn cancel = new Btn(GetText.tr("Cancel"), Btn.Kind.TONAL);
        cancel.addActionListener(e -> dialog.dispose());
        Btn ok = new Btn(GetText.tr("Add"), Btn.Kind.FILLED).icon(Icons.PLUS);
        ok.addActionListener(e -> {
            @SuppressWarnings("unchecked")
            ComboItem<String> item = (ComboItem<String>) box.getSelectedItem();
            List<String> chosen = new ArrayList<>();
            if (item == null || item.getValue() == null) {
                chosen.addAll(worlds);
            } else {
                chosen.add(item.getValue());
            }
            answer.add(chosen);
            dialog.dispose();
        });
        buttons.add(cancel);
        buttons.add(ok);
        root.add(buttons, BorderLayout.SOUTH);
        dialog.getRootPane().setDefaultButton(ok);

        dialog.setContentPane(root);
        dialog.pack();
        dialog.setMinimumSize(new Dimension(500, dialog.getHeight()));
        dialog.setLocationRelativeTo(owner);
        dialog.setVisible(true);
        return answer.isEmpty() ? null : answer.get(0);
    }
}
