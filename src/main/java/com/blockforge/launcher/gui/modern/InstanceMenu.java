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
import java.awt.Component;
import java.awt.Container;
import java.awt.Cursor;
import java.awt.Dimension;
import java.awt.FontMetrics;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.GridLayout;
import java.awt.RenderingHints;
import java.awt.event.KeyEvent;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.Path2D;
import java.awt.geom.RoundRectangle2D;
import java.util.ArrayList;
import java.util.List;

import javax.swing.BorderFactory;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.data.BackupMode;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.minecraft.loaders.LoaderType;
import com.blockforge.launcher.gui.dialogs.AddModsDialog;
import com.blockforge.launcher.gui.dialogs.EditModsDialog;
import com.blockforge.launcher.gui.dialogs.InstanceExportDialog;
import com.blockforge.launcher.snapshots.InstanceSnapshots;
import com.blockforge.launcher.snapshots.SnapshotService;
import com.blockforge.launcher.utils.OS;

/**
 * Everything else you can do with an instance, in a panel that drops down from its card: the instance at the top,
 * the most used actions as tiles, then the rest as rows. Groups open in place instead of in a second menu.
 */
final class InstanceMenu extends JPanel {
    private static final long serialVersionUID = 1L;

    static final int WIDTH = 296;

    private final Instance instance;
    private final Runnable changed;
    private Popover popover;
    private Item selected;
    private Group openGroup;

    private InstanceMenu(Instance instance, boolean hasUpdate, Runnable changed) {
        this.instance = instance;
        this.changed = changed;
        setOpaque(false);
        setLayout(new BoxLayout(this, BoxLayout.Y_AXIS));
        setBorder(BorderFactory.createEmptyBorder(6, 6, 6, 6));

        add(header());
        add(tiles());
        add(new Divider());

        add(new Row(Icons.PERSON, GetText.tr("Play offline with another name"),
            () -> InstanceActions.play(instance, true, false)));
        if (instance.isUpdatable()) {
            boolean canUpdate = hasUpdate && instance.launcher.isPlayable;
            Row update = new Row(Icons.REFRESH, GetText.tr("Update"), instance::update);
            update.setEnabled(canUpdate);
            if (canUpdate) {
                update.badge = GetText.tr("New");
            } else if (!hasUpdate) {
                update.hint = GetText.tr("Up to date");
            }
            add(update);
            add(new Row(Icons.DOWNLOAD, GetText.tr("Reinstall"), instance::startReinstall));
        }
        Group loader = loaderGroup();
        if (loader != null) {
            add(loader);
        }
        add(backupGroup());

        add(new Divider());
        add(new Row(Icons.IMAGE, GetText.tr("Change image"), () -> {
            instance.startChangeImage();
            changed.run();
        }));
        if (instance.canChangeDescription()) {
            add(new Row(Icons.NOTE, GetText.tr("Change description"), () -> {
                instance.startChangeDescription();
                changed.run();
            }));
        }
        add(moreGroup());

        add(new Divider());
        Row delete = new Row(Icons.TRASH, GetText.tr("Delete instance"), () -> InstanceActions.delete(instance));
        delete.danger = true;
        add(delete);
    }

    /** Drops the menu down from the instance's "more" button. */
    static void showBelow(Component button, Instance instance, boolean hasUpdate, Runnable changed) {
        InstanceMenu menu = new InstanceMenu(instance, hasUpdate, changed);
        menu.attach(Popover.below(button, menu));
    }

    /** Opens the menu where the instance was right clicked. */
    static void showAt(Component invoker, int x, int y, Instance instance, boolean hasUpdate, Runnable changed) {
        InstanceMenu menu = new InstanceMenu(instance, hasUpdate, changed);
        menu.attach(Popover.at(invoker, x, y, menu));
    }

    private void attach(Popover popover) {
        this.popover = popover;
        if (popover != null) {
            popover.onKey(this::key);
        }
    }

    @Override
    public Dimension getPreferredSize() {
        return new Dimension(WIDTH, super.getPreferredSize().height);
    }

    private JComponent header() {
        JPanel header = new JPanel(new GridLayout(2, 1, 0, 1));
        header.setOpaque(false);
        header.setBorder(BorderFactory.createEmptyBorder(6, 10, 10, 10));
        JLabel name = Lbl.text(instance.launcher.name, 14f, true, Palette::text);
        StringBuilder meta = new StringBuilder("Minecraft ").append(instance.id);
        if (instance.launcher.loaderVersion != null && instance.launcher.loaderVersion.type != null) {
            meta.append(" · ").append(instance.launcher.loaderVersion.type);
        }
        header.add(name);
        header.add(Lbl.text(meta.toString(), 12f, false, Palette::textDim));
        header.setAlignmentX(LEFT_ALIGNMENT);
        header.setMaximumSize(new Dimension(Integer.MAX_VALUE, header.getPreferredSize().height));
        // long names end in "..." instead of widening the menu
        header.setPreferredSize(new Dimension(WIDTH - 12, header.getPreferredSize().height));
        return header;
    }

    private JComponent tiles() {
        List<Tile> tiles = new ArrayList<>();
        tiles.add(new Tile(Icons.SETTINGS, GetText.tr("Settings"), () -> InstanceActions.openSettings(instance)));
        tiles.add(new Tile(Icons.EDIT, GetText.tr("Rename"), () -> {
            instance.startRename();
            changed.run();
        }));
        tiles.add(new Tile(Icons.COPY, GetText.tr("Clone"), instance::startClone));
        if (instance.canBeExported()) {
            tiles.add(new Tile(Icons.EXTERNAL, GetText.tr("Export"),
                () -> new InstanceExportDialog(instance).setVisible(true)));
        }

        JPanel row = new JPanel(new GridLayout(1, tiles.size(), 6, 0));
        row.setOpaque(false);
        row.setBorder(BorderFactory.createEmptyBorder(0, 2, 6, 2));
        for (Tile tile : tiles) {
            row.add(tile);
        }
        row.setAlignmentX(LEFT_ALIGNMENT);
        row.setMaximumSize(new Dimension(Integer.MAX_VALUE, row.getPreferredSize().height));
        return row;
    }

    private Group loaderGroup() {
        List<Row> rows = new ArrayList<>();
        String current = null;
        if (instance.launcher.loaderVersion == null) {
            for (LoaderType type : InstanceActions.addableLoaders(instance)) {
                rows.add(new Row(null, GetText.tr("Add {0}", type.toString()), () -> {
                    instance.addLoader(type);
                    changed.run();
                }));
            }
        } else {
            current = instance.launcher.loaderVersion.type;
            rows.add(new Row(null, GetText.tr("Change {0} version", current), () -> {
                instance.changeLoaderVersion();
                changed.run();
            }));
            rows.add(new Row(null, GetText.tr("Remove {0}", current), () -> {
                instance.removeLoader();
                changed.run();
            }));
        }
        if (rows.isEmpty()) {
            return null;
        }
        Group group = new Group(Icons.PUZZLE, GetText.tr("Mod loader"), rows);
        group.header.hint = current == null ? GetText.tr("None") : current;
        return group;
    }

    private Group backupGroup() {
        List<Row> rows = new ArrayList<>();
        rows.add(new Row(null, GetText.tr("Back up worlds and settings"), () -> instance.backup(BackupMode.NORMAL)));
        rows.add(new Row(null, GetText.tr("Back up worlds, settings and mods"),
            () -> instance.backup(BackupMode.NORMAL_PLUS_MODS)));
        rows.add(new Row(null, GetText.tr("Back up everything"), () -> instance.backup(BackupMode.FULL)));
        rows.add(new Row(null, GetText.tr("Save a snapshot now"), () -> InstanceActions.saveSnapshot(instance)));

        java.text.DateFormat format = java.text.DateFormat.getDateTimeInstance(java.text.DateFormat.MEDIUM,
            java.text.DateFormat.SHORT);
        for (InstanceSnapshots.Snapshot snapshot : InstanceSnapshots.latest(SnapshotService.folderFor(instance),
            6)) {
            Row restore = new Row(null, GetText.tr("Restore {0}", snapshot.label()),
                () -> InstanceActions.restoreSnapshot(instance, snapshot));
            restore.hint = format.format(new java.util.Date(snapshot.createdAt));
            rows.add(restore);
        }
        return new Group(Icons.ARCHIVE, GetText.tr("Backups and snapshots"), rows);
    }

    private Group moreGroup() {
        List<Row> rows = new ArrayList<>();
        // the classic windows still cover CurseForge, data packs, worlds and jar mods
        if (InstanceActions.modsViaPlatforms(instance)) {
            rows.add(new Row(null, GetText.tr("Add mods (CurseForge, data packs, worlds)"), () -> {
                new AddModsDialog(instance).setVisible(true);
                changed.run();
            }));
        }
        if (instance.launcher.enableEditingMods) {
            rows.add(new Row(null, GetText.tr("Edit mods (classic)"), () -> {
                new EditModsDialog(instance).setVisible(true);
                changed.run();
            }));
        }
        rows.add(new Row(null, GetText.tr("Create support bundle"), () -> SupportActions.createBundle(instance)));
        if (instance.hasWebsite()) {
            rows.add(new Row(null, GetText.tr("Open website"), () -> OS.openWebBrowser(instance.getWebsiteUrl())));
        }
        return new Group(Icons.MORE, GetText.tr("More"), rows);
    }

    /** Runs an action after the menu is gone, since most of them open a window of their own. */
    private void run(Runnable action) {
        Popover.closeOpen();
        SwingUtilities.invokeLater(action);
    }

    private void select(Item item) {
        if (selected == item) {
            return;
        }
        Item before = selected;
        selected = item;
        if (before != null) {
            before.repaint();
        }
        if (item != null) {
            item.repaint();
            item.scrollRectToVisible(new java.awt.Rectangle(0, 0, item.getWidth(), item.getHeight()));
        }
    }

    /** The items that can be reached with the arrow keys right now, top to bottom. */
    private List<Item> reachable() {
        List<Item> items = new ArrayList<>();
        collect(this, items);
        return items;
    }

    private static void collect(Container parent, List<Item> items) {
        for (Component c : parent.getComponents()) {
            if (!c.isVisible()) {
                continue;
            }
            if (c instanceof Item) {
                if (c.isEnabled()) {
                    items.add((Item) c);
                }
            } else if (c instanceof Container) {
                collect((Container) c, items);
            }
        }
    }

    private void key(KeyEvent e) {
        List<Item> items = reachable();
        if (items.isEmpty()) {
            return;
        }
        int at = items.indexOf(selected);
        switch (e.getKeyCode()) {
            case KeyEvent.VK_DOWN:
            case KeyEvent.VK_TAB:
                select(items.get(at < 0 ? 0 : (at + 1) % items.size()));
                e.consume();
                break;
            case KeyEvent.VK_UP:
                select(items.get(at <= 0 ? items.size() - 1 : at - 1));
                e.consume();
                break;
            case KeyEvent.VK_RIGHT:
                if (selected instanceof GroupHeader && !((GroupHeader) selected).group.isOpen()) {
                    selected.activate();
                    e.consume();
                }
                break;
            case KeyEvent.VK_LEFT:
                if (openGroup != null) {
                    GroupHeader header = openGroup.header;
                    openGroup.setOpen(false);
                    select(header);
                    e.consume();
                }
                break;
            case KeyEvent.VK_ENTER:
            case KeyEvent.VK_SPACE:
                if (selected != null) {
                    selected.activate();
                    e.consume();
                }
                break;
            default:
                break;
        }
    }

    /** Something in the menu that can be pointed at and used. */
    private abstract class Item extends JComponent {
        private static final long serialVersionUID = 1L;

        Item() {
            setCursor(Cursor.getPredefinedCursor(Cursor.HAND_CURSOR));
            addMouseListener(new MouseAdapter() {
                @Override
                public void mouseEntered(MouseEvent e) {
                    if (isEnabled()) {
                        select(Item.this);
                    }
                }

                @Override
                public void mouseExited(MouseEvent e) {
                    if (selected == Item.this) {
                        select(null);
                    }
                }

                @Override
                public void mouseReleased(MouseEvent e) {
                    if (isEnabled() && SwingUtilities.isLeftMouseButton(e) && contains(e.getPoint())) {
                        activate();
                    }
                }
            });
        }

        abstract void activate();

        @Override
        public void setEnabled(boolean enabled) {
            super.setEnabled(enabled);
            setCursor(Cursor.getPredefinedCursor(enabled ? Cursor.HAND_CURSOR : Cursor.DEFAULT_CURSOR));
        }

        boolean highlighted() {
            return selected == this && isEnabled();
        }

        Graphics2D smooth(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
            return g;
        }

        /** The text, cut with "..." when it doesn't fit in the width. */
        String fit(String text, FontMetrics metrics, int width) {
            if (metrics.stringWidth(text) <= width) {
                return text;
            }
            String dots = "...";
            int end = text.length();
            while (end > 0 && metrics.stringWidth(text.substring(0, end) + dots) > width) {
                end--;
            }
            return text.substring(0, end).trim() + dots;
        }
    }

    /** One of the big square buttons under the name. */
    private final class Tile extends Item {
        private static final long serialVersionUID = 1L;

        private final Icons icon;
        private final String label;
        private final Runnable action;

        Tile(Icons icon, String label, Runnable action) {
            this.icon = icon;
            this.label = label;
            this.action = action;
            setToolTipText(label);
        }

        @Override
        void activate() {
            run(action);
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(60, 58);
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = smooth(g0);
            int w = getWidth();
            int h = getHeight();
            // a light tint of the text colour, so the tiles show on the menu in both themes
            int tint = highlighted() ? (Palette.dark() ? 26 : 18) : (Palette.dark() ? 12 : 9);
            g.setColor(Palette.alpha(Palette.text(), tint));
            g.fill(new RoundRectangle2D.Float(0, 0, w, h, 16, 16));
            Color color = highlighted() ? Palette.accent() : Palette.text();
            icon.paint(g, (w - 20) / 2f, 10, 20, color);
            g.setFont(Palette.fontFor(label, 11.5f, true));
            FontMetrics metrics = g.getFontMetrics();
            String text = fit(label, metrics, w - 8);
            g.setColor(Palette.text());
            g.drawString(text, (w - metrics.stringWidth(text)) / 2, h - 11);
            g.dispose();
        }
    }

    /** A line in the menu: an icon, what it does, and maybe a note or a badge on the right. */
    private class Row extends Item {
        private static final long serialVersionUID = 1L;

        final Icons icon;
        final String label;
        final Runnable action;
        String hint;
        String badge;
        boolean danger;

        Row(Icons icon, String label, Runnable action) {
            this.icon = icon;
            this.label = label;
            this.action = action;
            setAlignmentX(LEFT_ALIGNMENT);
        }

        @Override
        void activate() {
            run(action);
        }

        /** rows inside a group have no icon and are a little shorter */
        boolean nested() {
            return icon == null;
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(WIDTH - 12, nested() ? 30 : 34);
        }

        @Override
        public Dimension getMaximumSize() {
            return new Dimension(Integer.MAX_VALUE, getPreferredSize().height);
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = smooth(g0);
            int w = getWidth();
            int h = getHeight();
            if (highlighted()) {
                g.setColor(danger ? Palette.alpha(Palette.danger(), Palette.dark() ? 34 : 26)
                    : Palette.alpha(Palette.text(), Palette.dark() ? 20 : 14));
                g.fill(new RoundRectangle2D.Float(0, 0, w, h, 12, 12));
            }

            Color color = !isEnabled() ? Palette.alpha(Palette.textDim(), 150)
                : danger ? Palette.danger() : Palette.text();
            if (icon != null) {
                icon.paint(g, 10, (h - 18) / 2f, 18, danger || !isEnabled() ? color
                    : highlighted() ? Palette.accent() : Palette.textDim());
            }

            int right = w - 12;
            right = paintTrailing(g, right, h);

            g.setFont(Palette.fontFor(label, nested() ? 12.5f : 13f, false));
            FontMetrics metrics = g.getFontMetrics();
            int x = 40;
            g.setColor(nested() && isEnabled() && !highlighted() ? Palette.mix(Palette.text(), Palette.textDim(), 0.35f)
                : color);
            g.drawString(fit(label, metrics, right - x - 8), x, (h + metrics.getAscent() - metrics.getDescent()) / 2);
            g.dispose();
        }

        /** Paints what goes on the right and returns where the label must end. */
        int paintTrailing(Graphics2D g, int right, int h) {
            if (badge != null) {
                g.setFont(Palette.bold(10.5f));
                FontMetrics metrics = g.getFontMetrics();
                int bw = metrics.stringWidth(badge) + 14;
                g.setColor(Palette.accent());
                g.fill(new RoundRectangle2D.Float(right - bw, (h - 18) / 2f, bw, 18, 18, 18));
                g.setColor(Palette.onAccent());
                g.drawString(badge, right - bw + 7, (h + metrics.getAscent() - metrics.getDescent()) / 2);
                right -= bw + 6;
            }
            if (hint != null) {
                g.setFont(Palette.fontFor(hint, 11.5f, false));
                FontMetrics metrics = g.getFontMetrics();
                String text = fit(hint, metrics, 120);
                int tw = metrics.stringWidth(text);
                g.setColor(Palette.textDim());
                g.drawString(text, right - tw, (h + metrics.getAscent() - metrics.getDescent()) / 2);
                right -= tw + 6;
            }
            return right;
        }
    }

    /** The row that opens and closes a group, with an arrow that points down while it is open. */
    private final class GroupHeader extends Row {
        private static final long serialVersionUID = 1L;

        final Group group;

        GroupHeader(Icons icon, String label, Group group) {
            super(icon, label, null);
            this.group = group;
        }

        @Override
        void activate() {
            group.setOpen(!group.isOpen());
        }

        @Override
        int paintTrailing(Graphics2D g, int right, int h) {
            Graphics2D arrow = (Graphics2D) g.create();
            arrow.translate(right - 6, h / 2f);
            if (group.isOpen()) {
                arrow.rotate(Math.PI / 2);
            }
            arrow.setColor(Palette.textDim());
            arrow.setStroke(new java.awt.BasicStroke(1.7f, java.awt.BasicStroke.CAP_ROUND,
                java.awt.BasicStroke.JOIN_ROUND));
            Path2D chevron = new Path2D.Float();
            chevron.moveTo(-2, -4);
            chevron.lineTo(2, 0);
            chevron.lineTo(-2, 4);
            arrow.draw(chevron);
            arrow.dispose();
            String saved = hint;
            hint = group.isOpen() ? null : saved;
            int end = super.paintTrailing(g, right - 18, h);
            hint = saved;
            return end;
        }
    }

    /** A row that opens to show more rows under it. Only one group is open at a time, to keep the menu short. */
    private final class Group extends JPanel {
        private static final long serialVersionUID = 1L;

        final GroupHeader header;
        private final JPanel body = new JPanel();

        Group(Icons icon, String label, List<Row> rows) {
            super(new BorderLayout());
            setOpaque(false);
            setAlignmentX(LEFT_ALIGNMENT);
            header = new GroupHeader(icon, label, this);
            add(header, BorderLayout.NORTH);

            body.setOpaque(false);
            body.setLayout(new BoxLayout(body, BoxLayout.Y_AXIS));
            body.setBorder(BorderFactory.createEmptyBorder(0, 0, 4, 0));
            for (Row row : rows) {
                body.add(row);
            }
            body.setVisible(false);
            add(body, BorderLayout.CENTER);
        }

        boolean isOpen() {
            return body.isVisible();
        }

        void setOpen(boolean open) {
            if (open == isOpen()) {
                return;
            }
            if (open && openGroup != null && openGroup != this) {
                openGroup.body.setVisible(false);
                openGroup.header.repaint();
            }
            body.setVisible(open);
            openGroup = open ? this : null;
            header.repaint();
            if (popover != null) {
                popover.relayout();
            }
        }

        @Override
        public Dimension getMaximumSize() {
            return new Dimension(Integer.MAX_VALUE, getPreferredSize().height);
        }
    }

    /** A thin line between sections. */
    private static final class Divider extends JComponent {
        private static final long serialVersionUID = 1L;

        Divider() {
            setAlignmentX(LEFT_ALIGNMENT);
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(10, 11);
        }

        @Override
        public Dimension getMaximumSize() {
            return new Dimension(Integer.MAX_VALUE, 11);
        }

        @Override
        protected void paintComponent(Graphics g) {
            g.setColor(Palette.outline());
            g.fillRect(8, getHeight() / 2, getWidth() - 16, 1);
        }
    }
}
