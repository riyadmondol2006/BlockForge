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
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.geom.RoundRectangle2D;
import java.awt.image.BufferedImage;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import javax.imageio.ImageIO;
import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JPopupMenu;
import javax.swing.JMenuItem;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.content.ContentKind;
import com.blockforge.launcher.content.ContentService;
import com.blockforge.launcher.data.DisableableMod;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.data.modrinth.ModrinthSearchHit;
import com.blockforge.launcher.data.modrinth.ModrinthVersion;
import com.blockforge.launcher.utils.OS;

/** The cards and rows the content manager is made of. */
final class ContentCards {
    private ContentCards() {
    }

    /** 12345678 as 12.3M, for download counts. */
    static String compact(long n) {
        if (n >= 1_000_000_000L) {
            return String.format(Locale.ROOT, "%.1fB", n / 1e9);
        }
        if (n >= 1_000_000L) {
            return String.format(Locale.ROOT, "%.1fM", n / 1e6);
        }
        if (n >= 1_000L) {
            return String.format(Locale.ROOT, "%.1fK", n / 1e3);
        }
        return String.valueOf(n);
    }

    static Icons iconFor(ContentKind kind) {
        switch (kind) {
            case RESOURCE_PACKS:
                return Icons.IMAGE;
            case SHADERS:
                return Icons.SPARKLE;
            default:
                return Icons.PUZZLE;
        }
    }

    /** A rounded square picture, with an icon until the picture has loaded. */
    static final class Picture extends JComponent {
        private static final long serialVersionUID = 1L;

        private final int size;
        private final Icons fallback;
        private BufferedImage image;

        Picture(int size, Icons fallback) {
            this.size = size;
            this.fallback = fallback;
            setOpaque(false);
        }

        void setImage(BufferedImage image) {
            this.image = image;
            repaint();
        }

        /** Loads a picture from the internet. */
        Picture load(String url) {
            if (url != null && !url.isEmpty()) {
                ImageCache.load(url, size * 2, size * 2, this::setImage);
            }
            return this;
        }

        /** Shows the pack.png inside a resource pack. */
        Picture loadPackImage(Path zip) {
            if (zip == null || !Files.isRegularFile(zip)) {
                return this;
            }
            String key = "packpng:" + zip + "#" + zip.toFile().lastModified();
            ImageCache.async(() -> {
                try (ZipFile file = new ZipFile(zip.toFile())) {
                    ZipEntry entry = file.getEntry("pack.png");
                    if (entry == null) {
                        return null;
                    }
                    try (InputStream in = file.getInputStream(entry)) {
                        BufferedImage source = ImageIO.read(in);
                        return source == null ? null : ImageCache.cover(source, size * 2, size * 2);
                    }
                }
            }, key, this::setImage);
            return this;
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(size, size);
        }

        @Override
        public Dimension getMinimumSize() {
            return getPreferredSize();
        }

        @Override
        public Dimension getMaximumSize() {
            return getPreferredSize();
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BILINEAR);
            float arc = size * 0.28f;
            RoundRectangle2D shape = new RoundRectangle2D.Float(0, 0, size, size, arc, arc);
            g.setColor(Palette.field());
            g.fill(shape);
            if (image != null) {
                // pixel art packs look best kept sharp
                g.setRenderingHint(RenderingHints.KEY_INTERPOLATION,
                    RenderingHints.VALUE_INTERPOLATION_NEAREST_NEIGHBOR);
                g.clip(shape);
                g.drawImage(image, 0, 0, size, size, null);
            } else {
                float s = size * 0.5f;
                fallback.paint(g, (size - s) / 2f, (size - s) / 2f, s, Palette.textDim());
            }
            g.dispose();
        }
    }

    /** One Modrinth search result with a one-click Install button. */
    static final class ResultCard extends RoundPanel {
        private static final long serialVersionUID = 1L;

        static final int WIDTH = 304;

        final String projectId;
        final InstallButton install = new InstallButton(128);

        ResultCard(ContentManagerDialog owner, ContentKind kind, ModrinthSearchHit hit) {
            super(new BorderLayout(0, 10), 20, Palette::card);
            this.projectId = hit.projectId;
            line(Palette::outline);
            hover(Palette::cardHover);
            pad(14, 14, 14, 14);
            setPreferredSize(new Dimension(WIDTH, 190));

            Picture picture = new Picture(52, iconFor(kind)).load(hit.iconUrl);

            JPanel names = new JPanel();
            names.setOpaque(false);
            names.setLayout(new BoxLayout(names, BoxLayout.Y_AXIS));
            // up to two lines, so packs whose names only differ at the end ("... Reimagined", "... Unbound") can
            // be told apart; the full name is in the tooltip too
            Lbl.Paragraph title = new Lbl.Paragraph(hit.title, 14.5f, 2, WIDTH - 140, Palette::text).bold();
            title.setToolTipText(hit.title);
            title.setMaximumSize(new Dimension(Integer.MAX_VALUE, title.getPreferredSize().height));

            JLabel author = Lbl.text(GetText.tr("by {0}", hit.author), 12f, false, Palette::textDim);
            title.setAlignmentX(LEFT_ALIGNMENT);
            author.setAlignmentX(LEFT_ALIGNMENT);
            names.add(Box.createVerticalGlue());
            names.add(title);
            names.add(Box.createVerticalStrut(2));
            names.add(author);
            names.add(Box.createVerticalGlue());

            Btn more = Btn.iconButton(Icons.MORE, GetText.tr("More options"));
            more.addActionListener(e -> {
                JPopupMenu menu = new JPopupMenu();
                JMenuItem versions = new JMenuItem(GetText.tr("Choose a version..."));
                versions.addActionListener(a -> owner.openVersionPicker(hit.projectId, kind));
                JMenuItem web = new JMenuItem(GetText.tr("Open on Modrinth"));
                web.addActionListener(a -> OS.openWebBrowser(modrinthUrl(kind, hit.slug)));
                menu.add(versions);
                menu.add(web);
                Popups.below(menu, more);
            });
            JPanel moreWrap = new JPanel(new BorderLayout());
            moreWrap.setOpaque(false);
            moreWrap.add(more, BorderLayout.NORTH);

            JPanel top = new JPanel(new BorderLayout(12, 0));
            top.setOpaque(false);
            top.add(picture, BorderLayout.WEST);
            top.add(names, BorderLayout.CENTER);
            top.add(moreWrap, BorderLayout.EAST);
            add(top, BorderLayout.NORTH);

            add(new Lbl.Paragraph(DiscoverPage.plain(hit.description), 12.5f, 2, WIDTH - 28, Palette::textDim),
                BorderLayout.CENTER);

            JLabel downloads = Lbl.text(compact(hit.downloads), 12f, true, Palette::textDim);
            downloads.setIcon(Icons.DOWNLOAD.asIcon(15, Palette::textDim));
            downloads.setIconTextGap(5);
            downloads.setToolTipText(GetText.tr("{0} downloads", String.format(Locale.ROOT, "%,d", hit.downloads)));
            install.addActionListener(e -> {
                if (install.getState() == InstallButton.State.READY
                    || install.getState() == InstallButton.State.FAILED) {
                    owner.requestInstall(hit.projectId, hit.title, kind);
                }
            });
            JPanel bottom = new JPanel(new BorderLayout());
            bottom.setOpaque(false);
            bottom.add(downloads, BorderLayout.WEST);
            bottom.add(install, BorderLayout.EAST);
            add(bottom, BorderLayout.SOUTH);

            owner.register(this);
        }
    }

    static String modrinthUrl(ContentKind kind, String slug) {
        String type = kind == ContentKind.SHADERS ? "shader"
            : kind == ContentKind.RESOURCE_PACKS ? "resourcepack" : "mod";
        return "https://modrinth.com/" + type + "/" + slug;
    }

    /** One installed mod or pack. */
    static final class Row extends RoundPanel {
        private static final long serialVersionUID = 1L;

        final DisableableMod mod;
        final Toggle toggle;
        final Btn update;
        private final JLabel name;

        Row(ContentManagerDialog owner, ContentKind kind, DisableableMod mod, ModrinthVersion newer,
            boolean showFile) {
            super(new BorderLayout(14, 0), 18, Palette::card);
            this.mod = mod;
            line(Palette::outline);
            hover(Palette::cardHover);
            pad(10, 12, 10, 10);
            setAlignmentX(LEFT_ALIGNMENT);
            setMaximumSize(new Dimension(Integer.MAX_VALUE, 64));
            setPreferredSize(new Dimension(400, 64));

            Instance instance = owner.service().instance();
            Picture picture = new Picture(42, iconFor(kind));
            if (mod.isFromModrinth() && mod.modrinthProject.iconUrl != null) {
                picture.load(mod.modrinthProject.iconUrl);
            } else if (kind == ContentKind.RESOURCE_PACKS) {
                try {
                    picture.loadPackImage(mod.getPath(instance));
                } catch (RuntimeException e) {
                    // no picture then
                }
            }
            add(picture, BorderLayout.WEST);

            JPanel text = new JPanel();
            text.setOpaque(false);
            text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
            name = Lbl.text(ContentService.nameOf(mod), 14f, true,
                () -> mod.isDisabled() ? Palette.textDim() : Palette.text());
            JLabel details = Lbl.text(showFile ? details(mod) + "  ·  " + mod.file : details(mod), 12f, false,
                Palette::textDim);
            name.setAlignmentX(LEFT_ALIGNMENT);
            details.setAlignmentX(LEFT_ALIGNMENT);
            text.add(Box.createVerticalGlue());
            text.add(name);
            text.add(Box.createVerticalStrut(2));
            text.add(details);
            text.add(Box.createVerticalGlue());
            add(text, BorderLayout.CENTER);

            JPanel actions = new JPanel(new FlowLayout(FlowLayout.RIGHT, 8, 0));
            actions.setOpaque(false);
            update = new Btn(GetText.tr("Update"), Btn.Kind.TONAL).icon(Icons.DOWNLOAD).compact();
            update.setVisible(newer != null);
            if (newer != null) {
                update.setToolTipText(GetText.tr("Update to {0}", newer.name));
                update.addActionListener(e -> owner.runUpdate(this, newer));
            }
            toggle = new Toggle(!mod.isDisabled(), GetText.tr("Turn {0} on or off", ContentService.nameOf(mod)));
            toggle.onChange(on -> owner.setEnabled(this, on));
            Btn remove = Btn.iconButton(Icons.TRASH, GetText.tr("Remove"));
            remove.addActionListener(e -> owner.remove(this));
            actions.add(update);
            actions.add(toggle);
            actions.add(remove);
            JPanel actionsWrap = new JPanel(new java.awt.GridBagLayout());
            actionsWrap.setOpaque(false);
            actionsWrap.add(actions);
            add(actionsWrap, BorderLayout.EAST);
        }

        private static String details(DisableableMod mod) {
            StringBuilder sb = new StringBuilder();
            String version = mod.version;
            if (mod.isFromModrinth() && mod.modrinthVersion.versionNumber != null) {
                version = mod.modrinthVersion.versionNumber;
            }
            if (version != null && !version.isEmpty() && !"Unknown".equalsIgnoreCase(version)) {
                sb.append(version).append("  ·  ");
            }
            if (mod.isFromModrinth()) {
                sb.append("Modrinth");
            } else if (mod.isFromCurseForge()) {
                sb.append("CurseForge");
            } else if (mod.isUserAdded()) {
                sb.append(GetText.tr("Added by you"));
            } else {
                sb.append(GetText.tr("Part of the modpack"));
            }
            if (mod.isDisabled()) {
                sb.append("  ·  ").append(GetText.tr("Off"));
            }
            return sb.toString();
        }
    }

    /** A centred block with a big icon, a heading, a line of text and some buttons, for empty and error states. */
    static JPanel message(Icons icon, String heading, String text, JComponent... actions) {
        JPanel box = new JPanel();
        box.setOpaque(false);
        box.setLayout(new BoxLayout(box, BoxLayout.Y_AXIS));
        box.setBorder(BorderFactory.createEmptyBorder(6, 24, 10, 24));

        RoundPanel badge = new RoundPanel(new java.awt.GridBagLayout(), 64, Palette::accentSoft);
        badge.setPreferredSize(new Dimension(64, 64));
        badge.setMaximumSize(new Dimension(64, 64));
        badge.add(new JLabel(icon.asIcon(30, Palette::accent)));
        badge.setAlignmentX(JComponent.CENTER_ALIGNMENT);
        box.add(badge);
        box.add(Box.createVerticalStrut(14));

        JLabel h = Lbl.text(heading, 17f, true, Palette::text);
        h.setAlignmentX(JComponent.CENTER_ALIGNMENT);
        box.add(h);
        if (text != null) {
            box.add(Box.createVerticalStrut(6));
            JLabel t = Lbl.text("<html><div style='text-align:center;width:420px'>" + escape(text) + "</div></html>",
                13f, false, Palette::textDim);
            t.setAlignmentX(JComponent.CENTER_ALIGNMENT);
            t.setHorizontalAlignment(JLabel.CENTER);
            box.add(t);
        }
        if (actions.length > 0) {
            box.add(Box.createVerticalStrut(16));
            JPanel row = new JPanel(new FlowLayout(FlowLayout.CENTER, 10, 0));
            row.setOpaque(false);
            for (JComponent action : actions) {
                row.add(action);
            }
            row.setAlignmentX(JComponent.CENTER_ALIGNMENT);
            row.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
            box.add(row);
        }
        return box;
    }

    static String escape(String text) {
        return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;");
    }
}
