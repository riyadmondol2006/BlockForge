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
import java.awt.Dimension;
import java.awt.FlowLayout;
import java.awt.GridBagConstraints;
import java.awt.GridBagLayout;
import java.awt.Window;
import java.awt.event.WindowAdapter;
import java.awt.event.WindowEvent;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Timer;
import java.util.TimerTask;

import javax.swing.BorderFactory;
import javax.swing.JButton;
import javax.swing.JDialog;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JTextField;
import javax.swing.JTextPane;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.builders.HTMLBuilder;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.data.json.Mod;
import com.blockforge.launcher.downloads.RestrictedFileMatcher;
import com.blockforge.launcher.gui.modern.Icons;
import com.blockforge.launcher.gui.modern.Lbl;
import com.blockforge.launcher.gui.modern.ModernDialogs;
import com.blockforge.launcher.gui.modern.Palette;
import com.blockforge.launcher.gui.layouts.WrapLayout;
import com.blockforge.launcher.managers.DialogManager;
import com.blockforge.launcher.network.Analytics;
import com.blockforge.launcher.utils.FileUtils;
import com.blockforge.launcher.utils.Hashing;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Utils;

public final class BrowserDownloadDialog extends JDialog {
    private final List<Mod> browserDownloadMods;
    public final List<Mod> modsDownloaded = new ArrayList<>();

    private final Timer timer = new Timer();

    private final JPanel topPanel = new JPanel(new BorderLayout());
    private final JPanel mainPanel = new JPanel();
    private final JPanel bottomPanel = new JPanel(new WrapLayout(WrapLayout.RIGHT, 8, 8));

    private JScrollPane mainScrollPanel = Utils.wrapInVerticalScroller(mainPanel, 16);
    private final JButton openFolderButton = ModernDialogs.secondary(GetText.tr("Open Folder")).icon(Icons.FOLDER);
    private final JButton openAllButton = ModernDialogs.primary(GetText.tr("Open All In Browser"))
            .icon(Icons.EXTERNAL);
    private final JButton skipRemainingButton = ModernDialogs.secondary(GetText.tr("Skip Remaining"));
    private final JButton cancelInstallButton = ModernDialogs.danger(GetText.tr("Cancel Install"));

    private final Path downloadPath = FileSystem.getUserDownloadsPath();

    public boolean success = false;

    public BrowserDownloadDialog(Window parent, List<Mod> browserDownloadMods) {
        super(parent, GetText.tr("Browser Download Mods"), ModalityType.DOCUMENT_MODAL);
        this.browserDownloadMods = browserDownloadMods;

        this.setPreferredSize(new Dimension(680, 600));
        this.setMinimumSize(new Dimension(680, 600));
        this.setResizable(true);
        this.setLayout(new BorderLayout());
        this.setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);

        checkForDownloadedMods(false);
        if (success) {
            return;
        }

        Analytics.sendScreenView("Browser Download Mods");

        setupComponents();

        setupFileWatching();

        this.setLocationRelativeTo(parent);
        this.setVisible(true);

        setDefaultCloseOperation(JDialog.DISPOSE_ON_CLOSE);
        addWindowListener(new WindowAdapter() {
            @Override
            public void windowClosed(WindowEvent arg0) {
                timer.cancel();
                timer.purge();
            }
        });
    }

    private void setupFileWatching() {
        timer.schedule(new TimerTask() {
            @Override
            public void run() {
                checkForDownloadedMods(true);
            }
        }, 5000, 5000);
    }

    /** whether a file's contents are the ones the mod is expected to have, by whichever hash is known */
    private static boolean contentsMatch(Mod mod, Path file) {
        if (mod.md5 != null) {
            return Hashing.md5(file).equals(Hashing.toHashCode(mod.md5));
        }
        if (mod.sha1 != null) {
            return Hashing.sha1(file).equals(Hashing.toHashCode(mod.sha1));
        }
        if (mod.sha512 != null) {
            return Hashing.sha512(file).equals(Hashing.toHashCode(mod.sha512));
        }
        return true;
    }

    private void checkForDownloadedMods(boolean allowReload) {
        boolean reloadMainPanel = false;
        List<Path> finishedDownloads = RestrictedFileMatcher.finishedFiles(downloadPath);

        for (Mod mod : browserDownloadMods) {
            if (modsDownloaded.contains(mod)) {
                continue;
            }

            // find the file wherever the browser put it and whatever it called it, then bring it in
            Path finalDestination = FileSystem.DOWNLOADS.resolve(mod.file);
            if (!Files.exists(finalDestination)) {
                RestrictedFileMatcher.find(finishedDownloads, mod.file, mod.filesize, file -> contentsMatch(mod, file))
                    .ifPresent(found -> FileUtils.moveFile(found, finalDestination, true));
            }

            Path downloadsPath = downloadPath.resolve(mod.file);
            Path downloadsPath2 = downloadPath.resolve(mod.file.replace(" ", "+"));
            Path finalLocation = FileSystem.DOWNLOADS.resolve(mod.file);
            Path finalLocation2 = FileSystem.DOWNLOADS.resolve(mod.file.replace(" ", "+"));

            // first check the user downloads folder (if not the same as final)
            if (!downloadsPath.equals(finalLocation) && Files.exists(downloadsPath)
                    && downloadsPath.toFile().length() == mod.filesize) {
                if (mod.md5 != null
                        && Hashing.md5(downloadsPath).equals(Hashing.toHashCode(mod.md5))) {
                    FileUtils.moveFile(downloadsPath, finalLocation, true);
                }

                if (mod.sha1 != null
                        && Hashing.sha1(downloadsPath).equals(Hashing.toHashCode(mod.sha1))) {
                    FileUtils.moveFile(downloadsPath, finalLocation, true);
                }

                if (mod.sha512 != null
                        && Hashing.sha512(downloadsPath).equals(Hashing.toHashCode(mod.sha512))) {
                    FileUtils.moveFile(downloadsPath, finalLocation, true);
                }
            }

            // first check the user downloads folder with spaces replaced (if not the same
            // as final)
            if (!downloadsPath2.equals(finalLocation2) && Files.exists(downloadsPath2)
                    && downloadsPath2.toFile().length() == mod.filesize) {
                if (mod.md5 != null
                        && Hashing.md5(downloadsPath2).equals(Hashing.toHashCode(mod.md5))) {
                    FileUtils.moveFile(downloadsPath2, finalLocation, true);
                }

                if (mod.sha1 != null
                        && Hashing.sha1(downloadsPath2).equals(Hashing.toHashCode(mod.sha1))) {
                    FileUtils.moveFile(downloadsPath2, finalLocation, true);
                }

                if (mod.sha512 != null
                        && Hashing.sha512(downloadsPath2).equals(Hashing.toHashCode(mod.sha512))) {
                    FileUtils.moveFile(downloadsPath2, finalLocation, true);
                }
            }

            if (Files.exists(finalLocation2)) {
                if (mod.md5 != null
                        && Hashing.md5(finalLocation2).equals(Hashing.toHashCode(mod.md5))) {
                    FileUtils.moveFile(finalLocation2, finalLocation, true);
                }

                if (mod.sha1 != null
                        && Hashing.sha1(finalLocation2).equals(Hashing.toHashCode(mod.sha1))) {
                    FileUtils.moveFile(finalLocation2, finalLocation, true);
                }

                if (mod.sha512 != null
                        && Hashing.sha512(finalLocation2).equals(Hashing.toHashCode(mod.sha512))) {
                    FileUtils.moveFile(finalLocation2, finalLocation, true);
                }
            }

            if (Files.exists(finalLocation)) {
                if (mod.md5 != null
                        && Hashing.md5(finalLocation).equals(Hashing.toHashCode(mod.md5))) {
                    modsDownloaded.add(mod);
                    reloadMainPanel = true;
                    continue;
                }

                if (mod.sha1 != null
                        && Hashing.sha1(finalLocation).equals(Hashing.toHashCode(mod.sha1))) {
                    modsDownloaded.add(mod);
                    reloadMainPanel = true;
                    continue;
                }

                if (mod.sha512 != null
                        && Hashing.sha512(finalLocation).equals(Hashing.toHashCode(mod.sha512))) {
                    modsDownloaded.add(mod);
                    reloadMainPanel = true;
                }
            }
        }

        if (modsDownloaded.size() == browserDownloadMods.size()) {
            close();
        }

        if (allowReload && reloadMainPanel) {
            setupMainPanel();
            revalidate();
            repaint();
        }
    }

    private void close() {
        success = true;

        timer.cancel();
        timer.purge();
        dispose();
    }

    private void setupComponents() {
        JTextPane explanationPane = new JTextPane();
        explanationPane.setContentType("text/html");
        explanationPane.setText(new HTMLBuilder().text(
                GetText.tr(
                        "In order to continue downloading this modpack, you must manually download the following mods.<br/>Simply click the Open button to open them in your browser, or alternatively you can copy the url to your clipboard to open in another browser.<br/><br/>Please download all files to <b>{0}</b> in order for the launcher to detect them.",
                        downloadPath.toAbsolutePath().toString()))
                .center().build());
        explanationPane.setEditable(false);
        explanationPane.setBackground(null);
        explanationPane.setBorder(null);
        explanationPane.setOpaque(false);
        this.topPanel.setOpaque(false);
        this.topPanel.add(ModernDialogs.header(getTitle(), null, Icons.DOWNLOAD), BorderLayout.NORTH);
        this.topPanel.add(explanationPane, BorderLayout.CENTER);
        this.topPanel.setBorder(BorderFactory.createEmptyBorder(0, 0, 14, 0));

        this.mainPanel.setLayout(new GridBagLayout());
        this.mainPanel.setBorder(BorderFactory.createEmptyBorder(8, 12, 8, 12));
        this.mainPanel.setOpaque(false);
        this.mainScrollPanel.setBorder(ModernDialogs.outline());
        this.mainScrollPanel.setOpaque(false);
        this.mainScrollPanel.getViewport().setOpaque(false);
        this.bottomPanel.setOpaque(false);
        this.bottomPanel.setBorder(BorderFactory.createEmptyBorder(10, 0, 0, 0));

        setupMainPanel();

        this.bottomPanel.add(openFolderButton);
        this.bottomPanel.add(openAllButton);
        this.bottomPanel.add(skipRemainingButton);
        this.bottomPanel.add(cancelInstallButton);

        openFolderButton.addActionListener(l -> OS.openFileExplorer(downloadPath));

        openAllButton.addActionListener(l -> {
            for (Mod mod : browserDownloadMods) {
                if (!modsDownloaded.contains(mod)) {
                    OS.openWebBrowser(mod.url);
                }
            }
        });

        skipRemainingButton.addActionListener(l -> {
            int ret = DialogManager.yesNoDialog().setTitle(GetText.tr("Are You Sure?"))
                    .setContent(new HTMLBuilder().center().text(GetText.tr(
                            "Skipping the remaining downloads may cause issues launching the modpack.<br/><br/>Are you sure you want to do this?"))
                            .build())
                    .setType(DialogManager.WARNING).show();

            if (ret == 0) {
                close();
            }
        });

        cancelInstallButton.addActionListener(l -> {
            timer.cancel();
            timer.purge();
            dispose();
        });

        ModernDialogs.install(this, this.topPanel, this.mainScrollPanel, this.bottomPanel);
    }

    private void setupMainPanel() {
        this.mainPanel.removeAll();

        GridBagConstraints gbc = new GridBagConstraints();
        gbc.gridx = 0;
        gbc.gridy = 0;

        gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        JLabel modLabel = Lbl.text("Mod", 12f, true, Palette::textDim);
        modLabel.setFont(App.THEME.getBoldFont());
        this.mainPanel.add(modLabel, gbc);

        gbc.gridx++;
        gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        JLabel filenameLabel = Lbl.text("Filename", 12f, true, Palette::textDim);
        filenameLabel.setFont(App.THEME.getBoldFont());
        this.mainPanel.add(filenameLabel, gbc);

        gbc.gridx++;
        gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
        gbc.anchor = GridBagConstraints.BASELINE_LEADING;
        JLabel actionsLabel = Lbl.text("Actions", 12f, true, Palette::textDim);
        actionsLabel.setFont(App.THEME.getBoldFont());
        this.mainPanel.add(actionsLabel, gbc);

        for (Mod mod : browserDownloadMods) {
            gbc.gridy++;
            gbc.gridx = 0;

            gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
            gbc.anchor = GridBagConstraints.BASELINE_LEADING;
            JTextField nameTextField = new JTextField(mod.name);
            nameTextField.setEditable(false);
            nameTextField.setBorder(null);
            nameTextField.setBackground(null);
            nameTextField.setPreferredSize(new Dimension(200, 33));
            this.mainPanel.add(nameTextField, gbc);

            gbc.gridx++;
            gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
            gbc.anchor = GridBagConstraints.BASELINE_LEADING;
            JTextField fileTextField = new JTextField(mod.file);
            fileTextField.setEditable(false);
            fileTextField.setBorder(null);
            fileTextField.setBackground(null);
            fileTextField.setPreferredSize(new Dimension(200, 33));
            this.mainPanel.add(fileTextField, gbc);

            gbc.gridx++;
            gbc.insets = UIConstants.LEFT_TO_RIGHT_SPACER;
            gbc.anchor = GridBagConstraints.BASELINE_LEADING;

            if (modsDownloaded.contains(mod)) {
                this.mainPanel.add(Lbl.text(GetText.tr("Download Complete"), 13f, true, Palette::accent), gbc);
            } else {
                JPanel buttonPanel = new JPanel(new FlowLayout(FlowLayout.LEFT, 6, 0));
                buttonPanel.setOpaque(false);
                JButton openButton = ModernDialogs.secondary(GetText.tr("Open")).compact();
                buttonPanel.add(openButton, gbc);
                openButton.addActionListener(l -> OS.openWebBrowser(mod.url));

                JButton copyLinkButton = ModernDialogs.secondary(GetText.tr("Copy Link")).compact();
                buttonPanel.add(copyLinkButton, gbc);
                copyLinkButton.addActionListener(l -> OS.copyToClipboard(mod.url));
                this.mainPanel.add(buttonPanel, gbc);
            }
        }

        gbc.gridy++;
        gbc.gridx = 0;
        gbc.gridwidth = GridBagConstraints.REMAINDER;
        gbc.weighty = 1;
        JPanel filler = new JPanel();
        filler.setOpaque(false);
        this.mainPanel.add(filler, gbc);
    }
}
