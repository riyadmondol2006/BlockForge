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
import java.awt.Dimension;
import java.awt.GradientPaint;
import java.awt.Graphics;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.event.MouseAdapter;
import java.awt.event.MouseEvent;
import java.awt.geom.Area;
import java.awt.geom.RoundRectangle2D;
import java.awt.image.BufferedImage;
import java.io.File;
import java.time.Duration;
import java.time.Instant;

import javax.swing.BorderFactory;
import javax.swing.BoxLayout;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.viewmodel.impl.InstancesTabViewModel;

import io.reactivex.rxjava3.disposables.Disposable;

/**
 * A card for one instance: its picture, name, what it runs and a Play button.
 */
public class InstanceTile extends RoundPanel {
    private static final long serialVersionUID = 1L;

    public static final int WIDTH = 272;
    public static final int HEIGHT = 268;

    /** shared so that update checks are cached between tiles */
    private static InstancesTabViewModel updateChecker;

    private final Instance instance;
    private final Banner banner = new Banner();
    private final JLabel nameLabel;
    private final JLabel metaLabel;
    private final JLabel playedLabel;
    private boolean hasUpdate;
    private Disposable updateSubscription;

    public InstanceTile(Instance instance, Runnable changed) {
        super(new BorderLayout(), 22, Palette::card);
        this.instance = instance;
        line(Palette::outline);
        hover(Palette::cardHover);
        setPreferredSize(new Dimension(WIDTH, HEIGHT));
        setCursor(Cursor.getPredefinedCursor(Cursor.DEFAULT_CURSOR));
        // files dropped on the card go into this instance
        putClientProperty(FileDropFlow.INSTANCE_KEY, instance);

        add(banner, BorderLayout.NORTH);

        nameLabel = Lbl.text(instance.launcher.name, 15f, true, Palette::text);
        metaLabel = Lbl.dim(meta());
        metaLabel.setFont(Palette.font(12f));
        playedLabel = Lbl.dim(lastPlayedText());
        playedLabel.setFont(Palette.font(12f));

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        nameLabel.setAlignmentX(LEFT_ALIGNMENT);
        metaLabel.setAlignmentX(LEFT_ALIGNMENT);
        playedLabel.setAlignmentX(LEFT_ALIGNMENT);
        text.add(nameLabel);
        text.add(javax.swing.Box.createVerticalStrut(2));
        text.add(metaLabel);
        text.add(playedLabel);

        Btn play = new Btn(GetText.tr("Play"), Btn.Kind.FILLED).icon(Icons.PLAY).compact();
        play.addActionListener(e -> InstanceActions.play(instance, false, hasUpdate));
        Btn more = Btn.iconButton(Icons.MORE, GetText.tr("More options"));
        more.addActionListener(e -> InstanceActions.showMenu(more, 0, more.getHeight(), instance, hasUpdate, changed));

        Btn mods = new Btn(GetText.tr("Mods"), Btn.Kind.TONAL).icon(Icons.PUZZLE).compact();
        mods.setToolTipText(GetText.tr("Add mods, resource packs and shaders"));
        mods.addActionListener(e -> InstanceActions.openContent(instance, com.blockforge.launcher.content.ContentKind.MODS,
            changed));

        JPanel left = new JPanel(new java.awt.FlowLayout(java.awt.FlowLayout.LEFT, 0, 0));
        left.setOpaque(false);
        left.add(play);
        left.add(javax.swing.Box.createHorizontalStrut(8));
        left.add(mods);

        JPanel actions = new JPanel(new BorderLayout());
        actions.setOpaque(false);
        actions.add(left, BorderLayout.WEST);
        actions.add(more, BorderLayout.EAST);

        JPanel body = new JPanel(new BorderLayout(0, 10));
        body.setOpaque(false);
        body.setBorder(BorderFactory.createEmptyBorder(12, 16, 14, 12));
        body.add(text, BorderLayout.CENTER);
        body.add(actions, BorderLayout.SOUTH);
        add(body, BorderLayout.CENTER);

        MouseAdapter mouse = new MouseAdapter() {
            @Override
            public void mouseClicked(MouseEvent e) {
                if (SwingUtilities.isLeftMouseButton(e) && e.getClickCount() == 2) {
                    InstanceActions.play(instance, false, hasUpdate);
                }
            }

            @Override
            public void mouseEntered(MouseEvent e) {
                // likely to be played next: get its files read ahead
                com.blockforge.launcher.mclauncher.GameFileWarmer.warmSoon(instance);
            }

            @Override
            public void mousePressed(MouseEvent e) {
                popup(e);
            }

            @Override
            public void mouseReleased(MouseEvent e) {
                popup(e);
            }

            private void popup(MouseEvent e) {
                if (e.isPopupTrigger()) {
                    InstanceActions.showMenu(e.getComponent(), e.getX(), e.getY(), instance, hasUpdate, changed);
                }
            }
        };
        banner.addMouseListener(mouse);
        text.addMouseListener(mouse);
        addMouseListener(mouse);

        loadImage();
    }

    public Instance getInstance() {
        return instance;
    }

    /** Re-reads the name, details and picture from the instance. */
    public void refresh() {
        nameLabel.setText(instance.launcher.name);
        metaLabel.setText(meta());
        playedLabel.setText(lastPlayedText());
        loadImage();
        repaint();
    }

    private String meta() {
        StringBuilder sb = new StringBuilder();
        if (instance.launcher.vanillaInstance || instance.launcher.pack == null
            || instance.launcher.pack.equals(instance.launcher.name)) {
            sb.append("Minecraft ").append(instance.id);
        } else {
            sb.append(instance.launcher.pack).append(" · ").append(instance.id);
        }
        if (instance.launcher.loaderVersion != null && instance.launcher.loaderVersion.type != null) {
            sb.append(" · ").append(instance.launcher.loaderVersion.type);
        }
        return sb.toString();
    }

    private String lastPlayedText() {
        Instant last = instance.getLastPlayedOrEpoch();
        if (instance.getNumberOfPlays() == 0 && last.equals(Instant.EPOCH)) {
            return GetText.tr("Never played");
        }
        if (last.equals(Instant.EPOCH)) {
            return GetText.tr("Played {0} times", instance.getNumberOfPlays());
        }
        return GetText.tr("Played {0}", ago(last));
    }

    public static String ago(Instant then) {
        Duration d = Duration.between(then, Instant.now());
        long minutes = d.toMinutes();
        if (minutes < 1) {
            return GetText.tr("just now");
        }
        if (minutes < 60) {
            return GetText.tr("{0} min ago", minutes);
        }
        long hours = d.toHours();
        if (hours < 24) {
            return GetText.tr("{0} h ago", hours);
        }
        long days = d.toDays();
        if (days < 30) {
            return days == 1 ? GetText.tr("yesterday") : GetText.tr("{0} days ago", days);
        }
        if (days < 365) {
            return GetText.tr("{0} months ago", days / 30);
        }
        return GetText.tr("over a year ago");
    }

    private void loadImage() {
        File custom = instance.getRoot().resolve("instance.png").toFile();
        String key = "instance:" + instance.getRoot() + "#" + custom.lastModified();
        ImageCache.async(() -> {
            BufferedImage source = ImageCache.toBuffered(instance.getImage());
            return source == null ? null : ImageCache.cover(source, ImageCache.px(WIDTH * 4 / 3), ImageCache.px(128));
        }, key, image -> {
            banner.image = image;
            banner.repaint();
        });
    }

    @Override
    public void addNotify() {
        super.addNotify();

        if (instance.launcher.vanillaInstance || updateSubscription != null) {
            return;
        }

        synchronized (InstanceTile.class) {
            if (updateChecker == null) {
                updateChecker = new InstancesTabViewModel();
            }
        }

        updateSubscription = updateChecker.getHasUpdateObservable(instance).subscribe(update -> SwingUtilities
            .invokeLater(() -> {
                hasUpdate = Boolean.TRUE.equals(update);
                banner.repaint();
            }), error -> {
                // update checks are best effort, offline use is fine
            });
    }

    @Override
    public void removeNotify() {
        if (updateSubscription != null) {
            updateSubscription.dispose();
            updateSubscription = null;
        }
        super.removeNotify();
    }

    /** The picture area at the top of the card. */
    private final class Banner extends JComponent {
        private static final long serialVersionUID = 1L;

        private BufferedImage image;

        Banner() {
            setOpaque(false);
        }

        @Override
        public Dimension getPreferredSize() {
            return new Dimension(WIDTH, 128);
        }

        @Override
        protected void paintComponent(Graphics g0) {
            Graphics2D g = (Graphics2D) g0.create();
            g.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON);
            g.setRenderingHint(RenderingHints.KEY_INTERPOLATION, RenderingHints.VALUE_INTERPOLATION_BICUBIC);
            int w = getWidth();
            int h = getHeight();

            // round only the top corners to follow the card
            Area clip = new Area(new RoundRectangle2D.Float(0, 0, w, h + 40, 22, 22));
            clip.intersect(new Area(new java.awt.Rectangle(0, 0, w, h)));
            // narrow the clip rather than replace it, or a card half scrolled out of view paints over its surroundings
            g.clip(clip);

            if (image != null) {
                // cards stretch to fill their row, so the picture is scaled to cover rather than squashed
                double scale = Math.max(w / (double) image.getWidth(), h / (double) image.getHeight());
                int iw = (int) Math.ceil(image.getWidth() * scale);
                int ih = (int) Math.ceil(image.getHeight() * scale);
                g.drawImage(image, (w - iw) / 2, (h - ih) / 2, iw, ih, null);
                g.setPaint(new GradientPaint(0, h * 0.5f, new Color(0, 0, 0, 0), 0, h, new Color(0, 0, 0, 90)));
                g.fillRect(0, 0, w, h);
            } else {
                g.setPaint(new GradientPaint(0, 0, Palette.accentSoft(), w, h, Palette.field()));
                g.fillRect(0, 0, w, h);
                Icons.CUBE.paint(g, (w - 44) / 2f, (h - 44) / 2f, 44, Palette.alpha(Palette.accent(), 150));
            }

            if (hasUpdate) {
                String label = GetText.tr("Update");
                g.setFont(Palette.bold(11f));
                int tw = g.getFontMetrics().stringWidth(label);
                int bw = tw + 20;
                g.setColor(Palette.accent());
                g.fillRoundRect(w - bw - 10, 10, bw, 22, 22, 22);
                g.setColor(Palette.onAccent());
                g.drawString(label, w - bw, 10 + 15);
            }
            g.dispose();
        }
    }
}
