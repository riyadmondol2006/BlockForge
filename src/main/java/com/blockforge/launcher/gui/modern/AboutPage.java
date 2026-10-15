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
import java.awt.Component;
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.Image;
import java.awt.Toolkit;
import java.awt.datatransfer.StringSelection;

import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.ImageIcon;
import javax.swing.JComponent;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.constants.Constants;
import com.blockforge.launcher.update.LauncherUpdates;
import com.blockforge.launcher.utils.Java;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Utils;

/**
 * What BlockForge is, which version is running, where it comes from and where its data lives. Also the place to
 * check for a newer version by hand.
 */
public class AboutPage extends Page {
    private static final long serialVersionUID = 1L;

    private static final String ATLAUNCHER_URL = "https://github.com/ATLauncher/ATLauncher";

    private final JLabel updateStatus = Lbl.text(" ", 12.5f, false, Palette::textDim);
    private Btn checkButton;

    public AboutPage() {
        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        header.setBorder(BorderFactory.createEmptyBorder(28, 36, 12, 36));
        JLabel title = Lbl.title(GetText.tr("About"));
        JLabel sub = Lbl.dim(GetText.tr("Version, credits and where things are kept"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        header.add(title);
        header.add(sub);
        add(header, BorderLayout.NORTH);

        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));
        column.setBorder(BorderFactory.createEmptyBorder(8, 36, 36, 36));

        column.add(hero());
        column.add(Box.createVerticalStrut(28));
        column.add(section(GetText.tr("WHAT IT DOES")));
        column.add(features());
        column.add(Box.createVerticalStrut(10));
        column.add(section(GetText.tr("BUILT ON ATLAUNCHER")));
        column.add(textCard(GetText.tr(
            "BlockForge is a fork of ATLauncher, released under the GPL-3.0 license. The launcher's core (modpack installs, mod loaders, instances, accounts and launching Minecraft) comes from ATLauncher and its contributors. BlockForge adds a new interface, offline play, faster start up, imports from other launchers, snapshots and crash help.\n\nBlockForge is not affiliated with or endorsed by ATLauncher, Mojang or Microsoft."),
            new LinkButton(GetText.tr("ATLauncher on GitHub"), ATLAUNCHER_URL),
            new LinkButton(GetText.tr("License"), Constants.GITHUB_URL + "/blob/master/LICENSE"),
            new LinkButton(GetText.tr("Third-party notices"), Constants.GITHUB_URL + "/blob/master/NOTICE.md")));
        column.add(Box.createVerticalStrut(22));
        column.add(section(GetText.tr("PRIVACY AND SERVICES")));
        column.add(textCard(GetText.tr(
            "Nothing about how you use BlockForge is sent unless you turn it on in Settings. To work it talks to Mojang and Microsoft, CurseForge, Modrinth, the mod loader sites, ATLauncher's servers for the modpack catalogue, and GitHub to look for BlockForge updates."),
            new LinkButton(GetText.tr("Privacy details"), Constants.GITHUB_URL + "/blob/master/PRIVACY.md")));
        column.add(Box.createVerticalStrut(22));
        column.add(section(GetText.tr("THIS COMPUTER")));
        column.add(systemCard());

        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(column, BorderLayout.NORTH);
        add(FitWidthPanel.scroll(fit), BorderLayout.CENTER);
    }

    private JComponent hero() {
        RoundPanel card = new RoundPanel(new BorderLayout(22, 0), 26, Palette::card);
        card.line(Palette::outline);
        card.pad(22, 24, 22, 24);
        card.setAlignmentX(Component.LEFT_ALIGNMENT);
        card.setMaximumSize(new Dimension(Integer.MAX_VALUE, 190));

        Image logo = Utils.getImage("/assets/image/icon.png");
        if (logo != null) {
            JLabel image = new JLabel(new ImageIcon(logo.getScaledInstance(104, 104, Image.SCALE_SMOOTH)));
            image.setVerticalAlignment(JLabel.CENTER);
            card.add(image, BorderLayout.WEST);
        }

        JPanel text = new JPanel();
        text.setOpaque(false);
        text.setLayout(new BoxLayout(text, BoxLayout.Y_AXIS));
        JLabel name = Lbl.text(Constants.LAUNCHER_NAME, 28f, true, Palette::text);
        JLabel version = Lbl.text(GetText.tr("Version {0}", LauncherUpdates.currentVersionText()), 14f, true,
            Palette::accent);
        Lbl.Paragraph tagline = new Lbl.Paragraph(GetText.tr(
            "A Minecraft launcher with a modern interface that opens fast, plays offline without an account, and keeps working when downloads or mod sites misbehave."),
            13f, 3, 520, Palette::textDim);
        for (JComponent c : new JComponent[] { name, version, tagline, updateStatus }) {
            c.setAlignmentX(Component.LEFT_ALIGNMENT);
        }
        text.add(name);
        text.add(version);
        text.add(Box.createVerticalStrut(6));
        text.add(tagline);
        text.add(Box.createVerticalStrut(12));

        JPanel buttons = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        buttons.setOpaque(false);
        buttons.setAlignmentX(Component.LEFT_ALIGNMENT);
        checkButton = new Btn(GetText.tr("Check for updates"), Btn.Kind.FILLED).icon(Icons.REFRESH).compact();
        checkButton.addActionListener(e -> checkForUpdates());
        buttons.add(checkButton);
        buttons.add(Box.createHorizontalStrut(8));
        buttons.add(new LinkButton(GetText.tr("GitHub"), Constants.GITHUB_URL));
        buttons.add(Box.createHorizontalStrut(8));
        buttons.add(new LinkButton(GetText.tr("Report a problem"), Constants.GITHUB_URL + "/issues"));
        buttons.add(Box.createHorizontalStrut(8));
        buttons.add(new LinkButton(GetText.tr("Releases"), Constants.GITHUB_URL + "/releases"));
        text.add(buttons);
        text.add(Box.createVerticalStrut(6));
        text.add(updateStatus);
        card.add(text, BorderLayout.CENTER);
        return card;
    }

    private static JComponent section(String text) {
        JLabel label = Lbl.text(text, 11f, true, Palette::textDim);
        label.setBorder(BorderFactory.createEmptyBorder(0, 4, 10, 0));
        label.setAlignmentX(Component.LEFT_ALIGNMENT);
        label.setMaximumSize(new Dimension(Integer.MAX_VALUE, 28));
        return label;
    }

    private JComponent features() {
        FitWidthPanel grid = new FitWidthPanel(16);
        grid.setAlignmentX(Component.LEFT_ALIGNMENT);
        grid.add(feature(Icons.PERSON, GetText.tr("Play without an account"),
            GetText.tr("Type a name to play offline, pick a saved player, or sign in with Microsoft.")));
        grid.add(feature(Icons.EXPLORE, GetText.tr("Modpacks from everywhere"),
            GetText.tr("ATLauncher, CurseForge, Modrinth, FTB and Technic packs in one place, or any version with Fabric, Forge, NeoForge or Quilt.")));
        grid.add(feature(Icons.IMPORT, GetText.tr("Bring your instances"),
            GetText.tr("Finds Prism, MultiMC, CurseForge, GDLauncher and ATLauncher instances and copies them in.")));
        grid.add(feature(Icons.REFRESH, GetText.tr("Safe updates"),
            GetText.tr("A snapshot of mods and configs is taken before every update or reinstall, ready to restore.")));
        grid.add(feature(Icons.WARNING, GetText.tr("Crash help"),
            GetText.tr("Says in plain words what most likely went wrong, and makes a support bundle with personal details removed.")));
        grid.add(feature(Icons.CHECK, GetText.tr("Private by default"),
            GetText.tr("Usage statistics and crash reports stay off unless you turn them on.")));
        return grid;
    }

    private RoundPanel feature(Icons icon, String title, String description) {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 10), 22, Palette::card);
        card.line(Palette::outline);
        card.pad(16, 18, 16, 18);
        card.setPreferredSize(new Dimension(268, 150));

        JPanel top = new JPanel(new BorderLayout(12, 0));
        top.setOpaque(false);
        top.add(new JLabel(icon.asIcon(22, Palette.accent())), BorderLayout.WEST);
        top.add(Lbl.text(title, 14.5f, true, Palette::text), BorderLayout.CENTER);
        card.add(top, BorderLayout.NORTH);
        card.add(new Lbl.Paragraph(description, 12.5f, 4, 232, Palette::textDim), BorderLayout.CENTER);
        return card;
    }

    private JComponent textCard(String text, Btn... links) {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 14), 22, Palette::card);
        card.line(Palette::outline);
        card.pad(18, 20, 16, 20);
        card.setAlignmentX(Component.LEFT_ALIGNMENT);

        Lbl.Paragraph paragraph = new Lbl.Paragraph(text, 13f, 12, 640, Palette::text);
        card.add(paragraph, BorderLayout.CENTER);
        if (links.length > 0) {
            JPanel row = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
            row.setOpaque(false);
            for (int i = 0; i < links.length; i++) {
                if (i > 0) {
                    row.add(Box.createHorizontalStrut(8));
                }
                row.add(links[i]);
            }
            card.add(row, BorderLayout.SOUTH);
        }
        card.setMaximumSize(new Dimension(Integer.MAX_VALUE, card.getPreferredSize().height));
        return card;
    }

    private JComponent systemCard() {
        String info = systemInfo();

        RoundPanel card = new RoundPanel(new BorderLayout(0, 14), 22, Palette::card);
        card.line(Palette::outline);
        card.pad(18, 20, 16, 20);
        card.setAlignmentX(Component.LEFT_ALIGNMENT);
        card.add(new Lbl.Paragraph(info, 12.5f, 8, 640, Palette::textDim), BorderLayout.CENTER);

        JPanel row = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        row.setOpaque(false);
        Btn folder = new Btn(GetText.tr("Open data folder"), Btn.Kind.TONAL).icon(Icons.FOLDER).compact();
        folder.addActionListener(e -> OS.openFileExplorer(FileSystem.BASE_DIR));
        Btn copy = new Btn(GetText.tr("Copy details"), Btn.Kind.TONAL).compact();
        copy.addActionListener(e -> {
            Toolkit.getDefaultToolkit().getSystemClipboard().setContents(
                new StringSelection(Constants.LAUNCHER_NAME + " " + LauncherUpdates.currentVersionText() + "\n" + info),
                null);
            copy.setText(GetText.tr("Copied"));
        });
        row.add(folder);
        row.add(Box.createHorizontalStrut(8));
        row.add(copy);
        card.add(row, BorderLayout.SOUTH);
        card.setMaximumSize(new Dimension(Integer.MAX_VALUE, card.getPreferredSize().height));
        return card;
    }

    private static String systemInfo() {
        StringBuilder sb = new StringBuilder();
        sb.append(GetText.tr("System: {0} {1} ({2})", OS.getName(), OS.getVersion(), System.getProperty("os.arch")))
            .append('\n');
        sb.append(GetText.tr("Launcher Java: {0} from {1}", Java.getLauncherJavaVersion(),
            System.getProperty("java.vendor", "?"))).append('\n');
        sb.append(GetText.tr("Data folder: {0}", FileSystem.BASE_DIR.toAbsolutePath())).append('\n');
        sb.append(GetText.tr("Installed with: {0}", installedWith()));
        return sb.toString();
    }

    private static String installedWith() {
        switch (LauncherUpdates.installKind()) {
            case WINDOWS_EXE:
                return GetText.tr("Windows installer");
            case MAC_DMG:
                return GetText.tr("macOS app");
            case DEB:
                return GetText.tr("Debian/Ubuntu package");
            case RPM:
                return GetText.tr("RPM package");
            case JAR:
                return GetText.tr("jar file");
            default:
                return GetText.tr("other (updates from the Releases page)");
        }
    }

    private void checkForUpdates() {
        checkButton.setEnabled(false);
        updateStatus.setForeground(Palette.textDim());
        updateStatus.setText(GetText.tr("Checking GitHub for a newer version..."));
        Thread thread = new Thread(() -> {
            LauncherUpdates.Result result = LauncherUpdates.check();
            SwingUtilities.invokeLater(() -> {
                checkButton.setEnabled(true);
                if (result.error != null) {
                    updateStatus.setText(GetText.tr("Couldn't check for updates: {0}", result.error));
                } else if (result.newer) {
                    updateStatus.setText(GetText.tr("Version {0} is available.", result.release.tagName));
                    UpdateDialog.show(App.launcherFrame, result.release);
                } else if (result.release == null) {
                    updateStatus.setText(GetText.tr("No releases have been published yet."));
                } else {
                    updateStatus.setText(GetText.tr("You're up to date."));
                }
            });
        }, "BlockForge update check");
        thread.setDaemon(true);
        thread.start();
    }

    /** A tonal button that opens a web page. */
    private static final class LinkButton extends Btn {
        private static final long serialVersionUID = 1L;

        LinkButton(String text, String url) {
            super(text, Btn.Kind.TONAL);
            icon(Icons.EXTERNAL);
            compact();
            addActionListener(e -> OS.openWebBrowser(url));
        }
    }

    @Override
    public void onShow() {
        if (App.settings != null && !App.settings.checkForLauncherUpdates) {
            updateStatus.setText(GetText.tr("Automatic update checks are off in Settings."));
        }
    }
}
