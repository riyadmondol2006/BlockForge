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
import java.awt.Dialog;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Frame;
import java.awt.Image;
import java.awt.Toolkit;
import java.awt.Window;
import java.awt.datatransfer.StringSelection;
import java.nio.file.Path;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicBoolean;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.ImageIcon;
import javax.swing.JComponent;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JProgressBar;
import javax.swing.JScrollPane;
import javax.swing.JTextArea;
import javax.swing.SwingUtilities;
import javax.swing.Timer;
import javax.swing.WindowConstants;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.update.LauncherUpdates;
import com.blockforge.launcher.update.Release;
import com.blockforge.launcher.update.UpdatePlan;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Utils;

/**
 * Tells the player a newer BlockForge is out, shows what changed, and installs it with one click: it downloads the
 * installer that fits this computer and starts it.
 */
public final class UpdateDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private final Release release;
    private final UpdatePlan.Kind kind;
    private final Release.Asset asset;
    private final AtomicBoolean cancelled = new AtomicBoolean();

    private final JPanel body = new JPanel(new BorderLayout());
    private final JPanel footer = new JPanel(new BorderLayout());
    private final JProgressBar bar = new JProgressBar(0, 1000);
    private final JLabel status = Lbl.text(" ", 12f, false, Palette::textDim);
    private Btn primary;
    private boolean downloading;

    private UpdateDialog(Frame owner, Release release) {
        super(owner, Constants.LAUNCHER_NAME, Dialog.ModalityType.APPLICATION_MODAL);
        this.release = release;
        this.kind = LauncherUpdates.installKind();
        this.asset = UpdatePlan.pickAsset(release, kind, System.getProperty("os.arch"));

        setDefaultCloseOperation(WindowConstants.DO_NOTHING_ON_CLOSE);
        addWindowListener(new java.awt.event.WindowAdapter() {
            @Override
            public void windowClosing(java.awt.event.WindowEvent e) {
                later();
            }
        });
        setIconImage(Utils.getImage("/assets/image/icon.png"));

        JPanel root = new JPanel(new BorderLayout(0, 18)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(28, 32, 24, 32));
        root.add(header(), BorderLayout.NORTH);

        body.setOpaque(false);
        body.add(notes(), BorderLayout.CENTER);
        root.add(body, BorderLayout.CENTER);

        footer.setOpaque(false);
        showChoiceButtons();
        root.add(footer, BorderLayout.SOUTH);

        setContentPane(root);
        setPreferredSize(new Dimension(620, 500));
        setMinimumSize(new Dimension(480, 380));
        pack();
        setLocationRelativeTo(owner);
    }

    private JComponent header() {
        JPanel row = new JPanel(new BorderLayout(16, 0));
        row.setOpaque(false);

        Image logo = Utils.getImage("/assets/image/icon.png");
        if (logo != null) {
            row.add(new JLabel(new ImageIcon(logo.getScaledInstance(56, 56, Image.SCALE_SMOOTH))), BorderLayout.WEST);
        }

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel title = Lbl.title(GetText.tr("A new version is ready"));
        JLabel sub = Lbl.dim(GetText.tr("BlockForge {0} is out. You have {1}.", versionOf(release),
            LauncherUpdates.currentVersionText()));
        title.setAlignmentX(Component.LEFT_ALIGNMENT);
        sub.setAlignmentX(Component.LEFT_ALIGNMENT);
        text.add(Box.createVerticalGlue());
        text.add(title);
        text.add(Box.createVerticalStrut(2));
        text.add(sub);
        text.add(Box.createVerticalGlue());
        row.add(text, BorderLayout.CENTER);
        return row;
    }

    private JComponent notes() {
        JPanel column = new JPanel(new BorderLayout(0, 8));
        column.setOpaque(false);
        column.add(Lbl.text(GetText.tr("WHAT'S NEW"), 11f, true, Palette::textDim), BorderLayout.NORTH);

        JTextArea area = new JTextArea(plainNotes(release.body)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.card();
            }

            @Override
            public Color getForeground() {
                return Palette.text();
            }
        };
        area.setEditable(false);
        area.setLineWrap(true);
        area.setWrapStyleWord(true);
        area.setFont(Palette.font(13f));
        area.setBorder(BorderFactory.createEmptyBorder(12, 14, 12, 14));
        area.setCaretPosition(0);
        // read-only notes: no blinking text cursor
        area.setFocusable(false);

        RoundPanel card = new RoundPanel(new BorderLayout(), 18, Palette::card);
        card.line(Palette::outline);
        card.pad(2, 2, 2, 2);
        JScrollPane scroll = FitWidthPanel.scroll(area);
        scroll.getViewport().setOpaque(true);
        card.add(scroll, BorderLayout.CENTER);
        column.add(card, BorderLayout.CENTER);
        return column;
    }

    private void showChoiceButtons() {
        footer.removeAll();

        JPanel left = new JPanel(new FlowLayout(FlowLayout.LEFT, 8, 0));
        left.setOpaque(false);
        Btn skip = new Btn(GetText.tr("Skip this version"), Btn.Kind.TEXT);
        skip.addActionListener(e -> skip());
        left.add(skip);
        if (release.htmlUrl != null) {
            Btn page = new Btn(GetText.tr("Release page"), Btn.Kind.TEXT).icon(Icons.EXTERNAL);
            page.addActionListener(e -> OS.openWebBrowser(release.htmlUrl));
            left.add(page);
        }

        JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        right.setOpaque(false);
        Btn later = new Btn(GetText.tr("Later"), Btn.Kind.TONAL);
        later.addActionListener(e -> later());
        right.add(later);

        boolean canInstall = asset != null && kind != UpdatePlan.Kind.PAGE_ONLY;
        primary = new Btn(canInstall ? GetText.tr("Update now") : GetText.tr("Get the update"), Btn.Kind.FILLED)
            .icon(canInstall ? Icons.DOWNLOAD : Icons.EXTERNAL);
        primary.addActionListener(e -> {
            if (canInstall) {
                startDownload();
            } else {
                OS.openWebBrowser(release.htmlUrl != null ? release.htmlUrl : Constants.GITHUB_URL + "/releases");
                dispose();
            }
        });
        right.add(primary);
        getRootPane().setDefaultButton(primary);

        footer.add(left, BorderLayout.WEST);
        footer.add(right, BorderLayout.EAST);
        footer.revalidate();
        footer.repaint();
    }

    private void startDownload() {
        downloading = true;
        cancelled.set(false);

        JPanel progress = new JPanel();
        progress.setOpaque(false);
        progress.setLayout(new BoxLayout(progress, BoxLayout.Y_AXIS));
        bar.setValue(0);
        bar.setIndeterminate(asset.size <= 0);
        bar.setAlignmentX(Component.LEFT_ALIGNMENT);
        bar.setMaximumSize(new Dimension(Integer.MAX_VALUE, 8));
        bar.setPreferredSize(new Dimension(100, 8));
        status.setText(GetText.tr("Downloading {0}...", asset.name));
        status.setAlignmentX(Component.LEFT_ALIGNMENT);
        progress.add(bar);
        progress.add(Box.createVerticalStrut(8));
        progress.add(status);

        footer.removeAll();
        footer.add(progress, BorderLayout.CENTER);
        JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        right.setOpaque(false);
        right.setBorder(BorderFactory.createEmptyBorder(0, 16, 0, 0));
        Btn cancel = new Btn(GetText.tr("Cancel"), Btn.Kind.TONAL);
        cancel.addActionListener(e -> cancelled.set(true));
        right.add(cancel);
        footer.add(right, BorderLayout.EAST);
        footer.revalidate();
        footer.repaint();

        Thread worker = new Thread(() -> {
            try {
                Path file = LauncherUpdates.download(asset, kind,
                    (done, total) -> SwingUtilities.invokeLater(() -> showProgress(done, total)), cancelled);
                SwingUtilities.invokeLater(() -> status.setText(GetText.tr("Starting the installer...")));
                boolean closing = LauncherUpdates.install(file, kind);
                if (!closing) {
                    SwingUtilities.invokeLater(() -> showLinuxFinish(file));
                }
            } catch (Exception e) {
                boolean wasCancelled = cancelled.get();
                if (!wasCancelled) {
                    LogManager.logStackTrace("Launcher update failed", e);
                }
                SwingUtilities.invokeLater(() -> {
                    downloading = false;
                    showChoiceButtons();
                    if (!wasCancelled) {
                        primary.setText(GetText.tr("Try again"));
                        footer.add(Lbl.text(GetText.tr("Update failed: {0}", String.valueOf(e.getMessage())), 12f,
                            false, Palette::danger), BorderLayout.NORTH);
                        footer.revalidate();
                    }
                });
            }
        }, "BlockForge update download");
        worker.setDaemon(true);
        worker.start();
    }

    private void showProgress(long done, long total) {
        if (total > 0) {
            bar.setIndeterminate(false);
            bar.setValue((int) Math.min(1000, done * 1000 / total));
            status.setText(GetText.tr("Downloading... {0} of {1}", megabytes(done), megabytes(total)));
        } else {
            status.setText(GetText.tr("Downloading... {0}", megabytes(done)));
        }
    }

    /** Linux packages are installed by the system's software installer, which keeps BlockForge open. */
    private void showLinuxFinish(Path file) {
        downloading = false;
        body.removeAll();

        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        Lbl.Paragraph info = new Lbl.Paragraph(GetText.tr(
            "The update was downloaded and your software installer should have opened. Finish installing there, then close and reopen BlockForge.\n\nIf no installer opened, run this in a terminal:"),
            13f, 8, 540, Palette::text);
        info.setAlignmentX(Component.LEFT_ALIGNMENT);
        column.add(info);
        column.add(Box.createVerticalStrut(12));

        String command = LauncherUpdates.linuxInstallCommand(file, kind);
        SearchBox field = new SearchBox("", false, 30);
        field.setText(command);
        field.setEditable(false);
        field.setAlignmentX(Component.LEFT_ALIGNMENT);
        field.setMaximumSize(new Dimension(Integer.MAX_VALUE, 44));
        column.add(field);
        body.add(column, BorderLayout.NORTH);
        body.revalidate();
        body.repaint();

        footer.removeAll();
        JPanel right = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        right.setOpaque(false);
        Btn copy = new Btn(GetText.tr("Copy command"), Btn.Kind.TONAL);
        copy.addActionListener(e -> {
            Toolkit.getDefaultToolkit().getSystemClipboard().setContents(new StringSelection(command), null);
            copy.setText(GetText.tr("Copied"));
        });
        Btn folder = new Btn(GetText.tr("Show file"), Btn.Kind.TONAL).icon(Icons.FOLDER);
        folder.addActionListener(e -> OS.openFileExplorer(file, true));
        Btn done = new Btn(GetText.tr("Done"), Btn.Kind.FILLED);
        done.addActionListener(e -> dispose());
        right.add(folder);
        right.add(copy);
        right.add(done);
        footer.add(right, BorderLayout.EAST);
        footer.revalidate();
        footer.repaint();
    }

    private void later() {
        if (downloading) {
            cancelled.set(true);
        }
        dispose();
    }

    private void skip() {
        App.settings.skippedLauncherUpdate = release.tagName;
        App.settings.save();
        dispose();
    }

    private static String megabytes(long bytes) {
        return String.format(Locale.ROOT, "%.1f MB", bytes / (1024.0 * 1024.0));
    }

    private static String versionOf(Release release) {
        String tag = release.tagName == null ? "" : release.tagName;
        return tag.startsWith("v") || tag.startsWith("V") ? tag.substring(1) : tag;
    }

    /** Turns GitHub's Markdown release notes into readable plain text. */
    static String plainNotes(String markdown) {
        if (markdown == null || markdown.trim().isEmpty()) {
            return GetText.tr("Bug fixes and improvements.");
        }
        StringBuilder out = new StringBuilder();
        for (String line : markdown.replace("\r", "").split("\n")) {
            String l = line.replaceAll("^\\s*#{1,6}\\s*", "").replaceAll("^(\\s*)[-*+]\\s+", "$1• ")
                .replaceAll("\\*\\*(.+?)\\*\\*", "$1").replaceAll("__(.+?)__", "$1").replaceAll("`([^`]*)`", "$1")
                .replaceAll("!?\\[([^\\]]*)\\]\\(([^)]*)\\)", "$1").replaceAll("<!--.*?-->", "");
            out.append(l).append('\n');
        }
        return out.toString().replaceAll("\n{3,}", "\n\n").trim();
    }

    /**
     * Shows the dialog for {@code release} once the main window is up and no other dialog (like "Who's playing?") is
     * open, so it never pops up on top of something the player is doing.
     */
    public static void offer(Release release) {
        SwingUtilities.invokeLater(() -> {
            Timer wait = new Timer(1000, null);
            wait.addActionListener(e -> {
                Frame owner = App.launcherFrame;
                if (owner == null || !owner.isShowing() || anotherDialogOpen()) {
                    return;
                }
                wait.stop();
                new UpdateDialog(owner, release).setVisible(true);
            });
            wait.setInitialDelay(0);
            wait.start();
        });
    }

    /** Opens the dialog right away, for when the player asked to check. */
    public static void show(Frame owner, Release release) {
        new UpdateDialog(owner, release).setVisible(true);
    }

    private static boolean anotherDialogOpen() {
        for (Window window : Window.getWindows()) {
            if (window instanceof Dialog && window.isShowing() && ((Dialog) window).isModal()) {
                return true;
            }
        }
        return false;
    }
}
