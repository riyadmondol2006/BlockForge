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
import java.util.function.Supplier;

import javax.swing.BorderFactory;
import javax.swing.BoxLayout;
import javax.swing.JLabel;
import javax.swing.JPanel;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.gui.tabs.tools.AbstractToolPanel;
import com.blockforge.launcher.gui.tabs.tools.DebugModePanel;
import com.blockforge.launcher.gui.tabs.tools.DownloadClearerToolPanel;
import com.blockforge.launcher.gui.tabs.tools.LibrariesDeleterToolPanel;
import com.blockforge.launcher.gui.tabs.tools.LogClearerToolPanel;
import com.blockforge.launcher.gui.tabs.tools.NetworkCheckerToolPanel;
import com.blockforge.launcher.gui.tabs.tools.SkinUpdaterToolPanel;
import com.blockforge.launcher.gui.tabs.tools.ToolsViewModel;

/**
 * Maintenance tools as cards with a short explanation of what each one does.
 */
public class ToolsPage extends Page {
    private static final long serialVersionUID = 1L;

    private final ToolsViewModel viewModel = new ToolsViewModel();

    public ToolsPage() {
        JPanel header = new JPanel();
        header.setOpaque(false);
        header.setLayout(new BoxLayout(header, BoxLayout.Y_AXIS));
        header.setBorder(BorderFactory.createEmptyBorder(28, 36, 12, 36));
        JLabel title = Lbl.title(GetText.tr("Tools"));
        JLabel sub = Lbl.dim(GetText.tr("Clean up, diagnose problems and keep things tidy"));
        title.setAlignmentX(LEFT_ALIGNMENT);
        sub.setAlignmentX(LEFT_ALIGNMENT);
        header.add(title);
        header.add(sub);
        add(header, BorderLayout.NORTH);

        FitWidthPanel grid = new FitWidthPanel(18);
        grid.add(tool(Icons.INFO, GetText.tr("Network checker"),
            GetText.tr("Tests your connection to the servers the launcher needs and shares the result so problems can be diagnosed."),
            GetText.tr("Run"), () -> new NetworkCheckerToolPanel(viewModel)));
        grid.add(tool(Icons.FOLDER, GetText.tr("Support bundle"),
            GetText.tr("Packs the launcher log and the last played instance's logs into one ZIP with names, tokens and IDs removed, ready to share."),
            GetText.tr("Create"), () -> SupportActions.createBundle(SupportActions.lastPlayedInstance())));
        grid.add(tool(Icons.TRASH, GetText.tr("Clear logs"),
            GetText.tr("Removes the logs the launcher has written, to free up space. Instance logs are kept."),
            GetText.tr("Clear"), () -> new LogClearerToolPanel(viewModel)));
        grid.add(tool(Icons.TERMINAL, GetText.tr("Debug mode"),
            GetText.tr("Restarts the launcher with extra logging, handy when you are chasing a problem."),
            GetText.tr("Restart"), () -> new DebugModePanel(viewModel)));
        grid.add(tool(Icons.DOWNLOAD, GetText.tr("Clear downloads"),
            GetText.tr("Deletes cached downloads. Instances are not touched, but new installs may take longer."),
            GetText.tr("Clear"), () -> new DownloadClearerToolPanel(viewModel)));
        grid.add(tool(Icons.PERSON, GetText.tr("Update skins"),
            GetText.tr("Downloads the latest skin for each of your Microsoft accounts."), GetText.tr("Update"),
            () -> new SkinUpdaterToolPanel(viewModel)));
        grid.add(tool(Icons.LIBRARY, GetText.tr("Delete libraries"),
            GetText.tr("Removes the libraries Minecraft uses. An instance may need a reinstall if they don't download again."),
            GetText.tr("Delete"), () -> new LibrariesDeleterToolPanel(viewModel)));

        JPanel wrap = new JPanel(new BorderLayout());
        wrap.setOpaque(false);
        wrap.setBorder(BorderFactory.createEmptyBorder(8, 36, 36, 36));
        wrap.add(grid, BorderLayout.NORTH);
        FitWidthPanel fit = new FitWidthPanel(0);
        fit.setLayout(new BorderLayout());
        fit.add(wrap, BorderLayout.CENTER);
        add(FitWidthPanel.scroll(fit), BorderLayout.CENTER);
    }

    private RoundPanel tool(Icons icon, String name, String description, String action,
        Supplier<AbstractToolPanel> factory) {
        return tool(icon, name, description, action, () -> factory.get().launch());
    }

    private RoundPanel tool(Icons icon, String name, String description, String action, Runnable run) {
        RoundPanel card = new RoundPanel(new BorderLayout(0, 12), 22, Palette::card);
        card.line(Palette::outline);
        card.hover(Palette::cardHover);
        card.pad(18, 20, 16, 20);
        card.setPreferredSize(new Dimension(284, 210));

        JPanel top = new JPanel(new BorderLayout(12, 0));
        top.setOpaque(false);
        top.add(new JLabel(icon.asIcon(26, Palette::accent)), BorderLayout.WEST);
        top.add(Lbl.text(name, 15f, true, Palette::text), BorderLayout.CENTER);
        card.add(top, BorderLayout.NORTH);
        card.add(new Lbl.Paragraph(description, 12.5f, 5, 244, Palette::textDim), BorderLayout.CENTER);

        Btn button = new Btn(action, Btn.Kind.TONAL).compact();
        button.addActionListener(e -> run.run());
        JPanel bottom = new JPanel(new BorderLayout());
        bottom.setOpaque(false);
        bottom.add(button, BorderLayout.WEST);
        card.add(bottom, BorderLayout.SOUTH);
        return card;
    }
}
