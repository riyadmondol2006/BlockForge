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
import java.awt.Window;
import java.awt.datatransfer.DataFlavor;
import java.awt.dnd.DnDConstants;
import java.awt.dnd.DropTarget;
import java.awt.dnd.DropTargetAdapter;
import java.awt.dnd.DropTargetDropEvent;
import java.awt.image.BufferedImage;
import java.io.File;
import java.util.List;
import java.util.Locale;

import javax.imageio.ImageIO;
import javax.swing.BorderFactory;
import javax.swing.Box;
import javax.swing.BoxLayout;
import javax.swing.JDialog;
import javax.swing.JFileChooser;
import javax.swing.JLabel;
import javax.swing.JPanel;
import javax.swing.SwingUtilities;
import javax.swing.WindowConstants;
import javax.swing.filechooser.FileNameExtensionFilter;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.FileSystem;
import com.blockforge.launcher.data.MicrosoftAccount;
import com.blockforge.launcher.gui.dialogs.LoginWithMicrosoftDialog;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.MojangAPIUtils;
import com.blockforge.launcher.utils.OS;
import com.blockforge.launcher.utils.Utils;

/**
 * Changes a player's skin: shows the current one, lets the player pick or drop a PNG, previews it with the chosen arm
 * width and uploads it. Offline players get an explanation instead, since skins are kept by Microsoft.
 */
public final class SkinDialog extends JDialog {
    private static final long serialVersionUID = 1L;

    private static final String SKIN_SITE = "https://www.minecraftskins.com/";

    private final MicrosoftAccount account;
    private final SkinPreview preview = new SkinPreview();
    private final JLabel fileLabel = Lbl.text(" ", 12.5f, false, Palette::textDim);
    /** whether the message below the choices is an error, which shows it in red */
    private boolean messageIsError;
    private final JLabel message = Lbl.text(" ", 12.5f, false,
        () -> messageIsError ? Palette.danger() : Palette.textDim());
    private final Chip classic = new Chip(GetText.tr("Classic arms"));
    private final Chip slim = new Chip(GetText.tr("Slim arms"));
    private Btn upload;

    private BufferedImage current;
    private File chosen;
    private BufferedImage chosenImage;

    private SkinDialog(Window owner, MicrosoftAccount account) {
        super(owner, GetText.tr("Change skin"), Dialog.ModalityType.APPLICATION_MODAL);
        this.account = account;
        setDefaultCloseOperation(WindowConstants.DISPOSE_ON_CLOSE);
        setIconImage(Utils.getImage("/assets/image/icon.png"));

        JPanel root = new JPanel(new BorderLayout(0, 20)) {
            private static final long serialVersionUID = 1L;

            @Override
            public Color getBackground() {
                return Palette.bg();
            }
        };
        root.setBorder(BorderFactory.createEmptyBorder(28, 32, 24, 32));

        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        JLabel title = Lbl.title(GetText.tr("Change skin"));
        JLabel sub = Lbl.dim(account.offline
            ? GetText.tr("{0} is an offline player", account.minecraftUsername)
            : GetText.tr("For {0}. Your skin shows in every game and on every server.", account.minecraftUsername));
        title.setAlignmentX(Component.LEFT_ALIGNMENT);
        sub.setAlignmentX(Component.LEFT_ALIGNMENT);
        header.add(title);
        header.add(sub);
        root.add(header, BorderLayout.NORTH);

        if (account.offline) {
            root.add(offlineBody(), BorderLayout.CENTER);
            root.add(offlineFooter(), BorderLayout.SOUTH);
            setPreferredSize(new Dimension(560, 330));
        } else {
            root.add(body(), BorderLayout.CENTER);
            root.add(footer(), BorderLayout.SOUTH);
            setPreferredSize(new Dimension(660, 540));
            loadCurrentSkin();
            installDropTarget(root);
        }

        setContentPane(root);
        pack();
        setResizable(false);
        setLocationRelativeTo(owner);
    }

    private JPanel body() {
        JPanel body = new JPanel(new BorderLayout(26, 0));
        body.setOpaque(false);
        preview.setPreferredSize(new Dimension(190, 300));
        body.add(preview, BorderLayout.WEST);

        JPanel column = new JPanel();
        column.setOpaque(false);
        column.setLayout(new BoxLayout(column, BoxLayout.Y_AXIS));

        JLabel step1 = Lbl.text(GetText.tr("1. Choose a skin"), 14f, true, Palette::text);
        Lbl.Paragraph hint1 = new Lbl.Paragraph(GetText.tr(
            "Pick a 64x64 PNG skin file, or drag one onto this window. Need one? Browse a skin site and download a PNG."),
            12.5f, 4, 360, Palette::textDim);
        JPanel chooseRow = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        chooseRow.setOpaque(false);
        Btn choose = new Btn(GetText.tr("Choose file"), Btn.Kind.TONAL).icon(Icons.FOLDER).compact();
        choose.addActionListener(e -> chooseFile());
        Btn browse = new Btn(GetText.tr("Find skins online"), Btn.Kind.TEXT).icon(Icons.EXTERNAL).compact();
        browse.addActionListener(e -> OS.openWebBrowser(SKIN_SITE));
        chooseRow.add(choose);
        chooseRow.add(Box.createHorizontalStrut(8));
        chooseRow.add(browse);

        JLabel step2 = Lbl.text(GetText.tr("2. Arm width"), 14f, true, Palette::text);
        Lbl.Paragraph hint2 = new Lbl.Paragraph(GetText.tr(
            "Skins are made for one of two arm widths. Pick the one that looks right in the preview."), 12.5f, 3, 360,
            Palette::textDim);
        JPanel modelRow = new JPanel(new FlowLayout(FlowLayout.LEFT, 0, 0));
        modelRow.setOpaque(false);
        classic.setSelected(true);
        classic.onClick(() -> setSlim(false));
        slim.onClick(() -> setSlim(true));
        modelRow.add(classic);
        modelRow.add(Box.createHorizontalStrut(8));
        modelRow.add(slim);

        for (Component c : new Component[] { step1, hint1, chooseRow, fileLabel, step2, hint2, modelRow, message }) {
            ((javax.swing.JComponent) c).setAlignmentX(Component.LEFT_ALIGNMENT);
        }
        column.add(step1);
        column.add(Box.createVerticalStrut(4));
        column.add(hint1);
        column.add(Box.createVerticalStrut(10));
        column.add(chooseRow);
        column.add(Box.createVerticalStrut(6));
        column.add(fileLabel);
        column.add(Box.createVerticalStrut(20));
        column.add(step2);
        column.add(Box.createVerticalStrut(4));
        column.add(hint2);
        column.add(Box.createVerticalStrut(10));
        column.add(modelRow);
        column.add(Box.createVerticalStrut(18));
        column.add(message);
        body.add(column, BorderLayout.CENTER);
        return body;
    }

    private JPanel footer() {
        JPanel footer = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        footer.setOpaque(false);
        Btn cancel = new Btn(GetText.tr("Cancel"), Btn.Kind.TONAL);
        cancel.addActionListener(e -> dispose());
        upload = new Btn(GetText.tr("Use this skin"), Btn.Kind.FILLED).icon(Icons.CHECK);
        upload.setEnabled(false);
        upload.addActionListener(e -> upload());
        footer.add(cancel);
        footer.add(upload);
        return footer;
    }

    private JPanel offlineBody() {
        JPanel body = new JPanel(new BorderLayout(18, 0));
        body.setOpaque(false);
        body.add(new JLabel(Icons.INFO.asIcon(30, Palette.accent())), BorderLayout.WEST);
        body.add(new Lbl.Paragraph(GetText.tr(
            "Minecraft keeps skins on Microsoft's servers, so offline players always look like one of the default characters in game.\n\nTo use your own skin, sign in with the Microsoft account that owns Minecraft, then choose Change skin again."),
            13f, 8, 420, Palette::text), BorderLayout.CENTER);
        return body;
    }

    private JPanel offlineFooter() {
        JPanel footer = new JPanel(new FlowLayout(FlowLayout.RIGHT, 10, 0));
        footer.setOpaque(false);
        Btn close = new Btn(GetText.tr("Close"), Btn.Kind.TONAL);
        close.addActionListener(e -> dispose());
        Btn signIn = new Btn(GetText.tr("Sign in with Microsoft"), Btn.Kind.FILLED).icon(Icons.PERSON);
        signIn.addActionListener(e -> {
            dispose();
            LoginWithMicrosoftDialog login = new LoginWithMicrosoftDialog();
            login.setVisible(true);
            if (login.account != null) {
                login.account.updateSkin();
            }
        });
        footer.add(close);
        footer.add(signIn);
        return footer;
    }

    private void loadCurrentSkin() {
        File file = FileSystem.SKINS.resolve(account.getUUIDNoDashes() + ".png").toFile();
        try {
            if (file.isFile()) {
                current = ImageIO.read(file);
            }
        } catch (Exception e) {
            LogManager.debug("Couldn't read the current skin: " + e.getMessage());
        }
        if (current == null) {
            try (java.io.InputStream in = SkinDialog.class.getResourceAsStream("/assets/image/skins/default.png")) {
                current = in == null ? null : ImageIO.read(in);
            } catch (Exception e) {
                LogManager.debug("Couldn't read the default skin: " + e.getMessage());
            }
        }
        fileLabel.setText(GetText.tr("Showing your current skin"));
        preview.setSkin(current, false);
    }

    private void chooseFile() {
        JFileChooser chooser = new JFileChooser();
        File downloads = new File(System.getProperty("user.home"), "Downloads");
        chooser.setCurrentDirectory(downloads.isDirectory() ? downloads : FileSystem.SKINS.toFile());
        chooser.setDialogTitle(GetText.tr("Choose a skin"));
        chooser.setFileSelectionMode(JFileChooser.FILES_ONLY);
        chooser.setAcceptAllFileFilterUsed(false);
        chooser.setFileFilter(new FileNameExtensionFilter(GetText.tr("Skin image (.png)"), "png"));
        if (chooser.showOpenDialog(this) == JFileChooser.APPROVE_OPTION) {
            useFile(chooser.getSelectedFile());
        }
    }

    private void useFile(File file) {
        BufferedImage image = null;
        try {
            if (file.getName().toLowerCase(Locale.ROOT).endsWith(".png")) {
                image = ImageIO.read(file);
            }
        } catch (Exception e) {
            LogManager.debug("Couldn't read skin " + file + ": " + e.getMessage());
        }

        if (!SkinPreview.isValidSkin(image)) {
            showMessage(GetText.tr("That file isn't a skin. Skins are PNG images of 64x64 (or 64x32) pixels."), true);
            return;
        }

        chosen = file;
        chosenImage = image;
        fileLabel.setText(file.getName());
        preview.setSkin(image, slim.isSelected());
        upload.setEnabled(true);
        showMessage(" ", false);
    }

    private void setSlim(boolean on) {
        classic.setSelected(!on);
        slim.setSelected(on);
        preview.setSkin(chosenImage != null ? chosenImage : current, on);
    }

    private void upload() {
        if (chosen == null) {
            return;
        }
        upload.setEnabled(false);
        showMessage(GetText.tr("Uploading your skin..."), false);
        String variant = slim.isSelected() ? "slim" : "classic";
        Thread thread = new Thread(() -> {
            boolean ok;
            try {
                account.changeSkinPreCheck();
                ok = MojangAPIUtils.uploadSkin(account, chosen, variant);
                if (ok) {
                    account.updateSkin();
                    AccountManager.saveAccounts();
                }
            } catch (Exception e) {
                LogManager.logStackTrace("Error changing skin", e);
                ok = false;
            }
            boolean success = ok;
            SwingUtilities.invokeLater(() -> {
                if (success) {
                    if (App.launcherFrame != null) {
                        App.launcherFrame.repaint();
                    }
                    dispose();
                } else {
                    upload.setEnabled(true);
                    showMessage(GetText.tr(
                        "The skin couldn't be uploaded. Check your connection and try again, or sign in again if it keeps failing."),
                        true);
                }
            });
        }, "BlockForge skin upload");
        thread.setDaemon(true);
        thread.start();
    }

    private void showMessage(String text, boolean error) {
        messageIsError = error;
        message.setText(text);
        message.repaint();
    }

    private void installDropTarget(Component target) {
        new DropTarget(target, DnDConstants.ACTION_COPY, new DropTargetAdapter() {
            @Override
            public void drop(DropTargetDropEvent event) {
                try {
                    event.acceptDrop(DnDConstants.ACTION_COPY);
                    Object data = event.getTransferable().getTransferData(DataFlavor.javaFileListFlavor);
                    if (data instanceof List && !((List<?>) data).isEmpty() && ((List<?>) data).get(0) instanceof File) {
                        useFile((File) ((List<?>) data).get(0));
                    }
                    event.dropComplete(true);
                } catch (Exception e) {
                    event.dropComplete(false);
                }
            }
        }, true);
    }

    /** Opens the skin changer for {@code account}, or for the selected player when null. */
    public static void open(MicrosoftAccount account) {
        MicrosoftAccount target = account != null ? account : AccountManager.getSelectedAccount();
        Window owner = App.launcherFrame;
        if (target == null) {
            WhoIsPlayingDialog.askIfNeeded(App.launcherFrame);
            return;
        }
        new SkinDialog(owner, target).setVisible(true);
    }
}
