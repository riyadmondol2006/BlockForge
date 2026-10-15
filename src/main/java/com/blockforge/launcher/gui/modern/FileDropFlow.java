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

import java.awt.Component;
import java.awt.Window;
import java.io.File;
import java.lang.reflect.InvocationTargetException;
import java.util.ArrayList;
import java.util.EnumMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

import javax.swing.JComponent;
import javax.swing.SwingUtilities;

import org.mini2Dx.gettext.GetText;

import com.blockforge.launcher.App;
import com.blockforge.launcher.constants.UIConstants;
import com.blockforge.launcher.content.ContentKind;
import com.blockforge.launcher.content.ContentService;
import com.blockforge.launcher.content.DroppedFile;
import com.blockforge.launcher.content.DroppedFileDetector;
import com.blockforge.launcher.content.LoaderSupport;
import com.blockforge.launcher.data.Instance;
import com.blockforge.launcher.gui.dialogs.ProgressDialog;
import com.blockforge.launcher.managers.AccountManager;
import com.blockforge.launcher.managers.LogManager;
import com.blockforge.launcher.utils.ImportPackUtils;

/**
 * What happens to files dropped on the launcher: each is looked at to see what it is, then mods, resource packs and
 * shaders go into an instance (the one they were dropped on, or one the player picks), data packs into a world, and
 * modpacks become new instances through the normal import.
 */
public final class FileDropFlow {
    /** Components showing an instance carry it under this client property, so a drop on them goes there. */
    public static final String INSTANCE_KEY = "blockforge.dropInstance";

    private FileDropFlow() {
    }

    /** The instance shown by a component or one of its parents, or null. */
    public static Instance instanceAt(Component component) {
        for (Component c = component; c != null; c = c.getParent()) {
            if (c instanceof JComponent) {
                Object value = ((JComponent) c).getClientProperty(INSTANCE_KEY);
                if (value instanceof Instance) {
                    return (Instance) value;
                }
            }
        }
        return null;
    }

    /** The component carrying the instance at or above a component, for outlining it. */
    public static JComponent instanceComponentAt(Component component) {
        for (Component c = component; c != null; c = c.getParent()) {
            if (c instanceof JComponent && ((JComponent) c).getClientProperty(INSTANCE_KEY) instanceof Instance) {
                return (JComponent) c;
            }
        }
        return null;
    }

    /**
     * Handles dropped files.
     *
     * @param target  the instance they were dropped on, or null to ask
     * @param changed runs with the instance once files went into it
     */
    public static void handle(Window owner, List<File> files, Instance target, Consumer<Instance> changed) {
        Thread detect = new Thread(() -> {
            List<DroppedFile> found = new ArrayList<>();
            for (File file : files) {
                if (file != null) {
                    found.add(DroppedFileDetector.detect(file.toPath()));
                }
            }
            SwingUtilities.invokeLater(() -> handleDetected(owner, found, target, changed));
        }, "dropped-files");
        detect.setDaemon(true);
        detect.start();
    }

    /** The second half of {@link #handle}, once every file has been looked at. Runs on the interface thread. */
    static void handleDetected(Window owner, List<DroppedFile> found, Instance target, Consumer<Instance> changed) {
        List<DroppedFile> content = new ArrayList<>();
        List<DroppedFile> modpacks = new ArrayList<>();
        List<String> unknown = new ArrayList<>();
        for (DroppedFile file : found) {
            if (file.kind.isModpack()) {
                modpacks.add(file);
            } else if (file.kind.goesIntoInstance()) {
                content.add(file);
            } else {
                unknown.add(file.fileName);
            }
        }

        for (DroppedFile pack : modpacks) {
            offerImport(owner, pack);
        }

        if (content.isEmpty()) {
            showUnknown(owner, unknown);
            return;
        }

        Instance instance = target;
        if (instance == null) {
            InstancePickerDialog.Choice choice = InstancePickerDialog.pick(owner, content);
            if (choice == null) {
                return;
            }
            if (choice.newInstance) {
                App.navigate(UIConstants.LAUNCHER_CREATE_PACK_TAB);
                toast(GetText.tr("Make the new instance, then drop the files on it"));
                return;
            }
            instance = choice.instance;
        }

        ContentService service = new ContentService(instance);
        List<String> worlds = new ArrayList<>();
        int dataPacks = count(content, DroppedFile.Kind.DATA_PACK);
        if (dataPacks > 0) {
            List<String> all = service.worlds();
            List<String> chosen = null;
            if (all.isEmpty()) {
                ChoiceDialog.ask(owner, Icons.INFO, GetText.tr("Data packs need a world"),
                    GetText.tr(
                        "Data packs are added to a world, and {0} has none yet. Play it once to make a world, then drop the data pack again.",
                        instance.launcher.name),
                    GetText.tr("OK"));
            } else {
                chosen = InstancePickerDialog.pickWorlds(owner, instance, all, dataPacks);
            }
            if (chosen == null) {
                content.removeIf(f -> f.kind == DroppedFile.Kind.DATA_PACK);
            } else {
                worlds.addAll(chosen);
            }
        }
        if (content.isEmpty()) {
            showUnknown(owner, unknown);
            return;
        }

        Instance into = instance;
        Thread worker = new Thread(() -> {
            ContentService.AddFilesResult result = service.addDropped(content, worlds,
                (file, where) -> askConflict(owner, file, where));
            SwingUtilities.invokeLater(() -> finish(owner, into, service, result, unknown, changed));
        }, "dropped-files-copy");
        worker.setDaemon(true);
        worker.start();
    }

    private static void finish(Window owner, Instance instance, ContentService service,
        ContentService.AddFilesResult result, List<String> unknown, Consumer<Instance> changed) {
        if (result.addedCount() > 0) {
            toast(GetText.tr("Added {0} to {1}", summary(result), instance.launcher.name));
            if (changed != null) {
                changed.accept(instance);
            }
        }
        List<String> notAdded = new ArrayList<>(unknown);
        notAdded.addAll(result.rejected);
        showUnknown(owner, notAdded);

        if (result.added.containsKey(ContentKind.SHADERS)) {
            offerShaderLoader(owner, instance, service, changed);
        }
    }

    /** "2 mods and 1 shader". */
    static String summary(ContentService.AddFilesResult result) {
        Map<ContentKind, Integer> counts = new EnumMap<>(ContentKind.class);
        for (Map.Entry<ContentKind, List<String>> e : result.added.entrySet()) {
            counts.put(e.getKey(), e.getValue().size());
        }
        List<String> parts = new ArrayList<>();
        for (Map.Entry<ContentKind, Integer> e : counts.entrySet()) {
            parts.add(count(e.getValue(), e.getKey()));
        }
        if (!result.dataPacks.isEmpty()) {
            parts.add(result.dataPacks.size() == 1 ? GetText.tr("1 data pack")
                : GetText.tr("{0} data packs", result.dataPacks.size()));
        }
        return join(parts);
    }

    private static String count(int n, ContentKind kind) {
        switch (kind) {
            case RESOURCE_PACKS:
                return n == 1 ? GetText.tr("1 resource pack") : GetText.tr("{0} resource packs", n);
            case SHADERS:
                return n == 1 ? GetText.tr("1 shader") : GetText.tr("{0} shaders", n);
            default:
                return n == 1 ? GetText.tr("1 mod") : GetText.tr("{0} mods", n);
        }
    }

    /** What a set of dropped files is, for headings: "2 mods, 1 resource pack and 1 data pack". */
    static String describe(List<DroppedFile> files) {
        Map<DroppedFile.Kind, Integer> counts = new EnumMap<>(DroppedFile.Kind.class);
        for (DroppedFile f : files) {
            counts.merge(f.kind, 1, Integer::sum);
        }
        List<String> parts = new ArrayList<>();
        for (Map.Entry<DroppedFile.Kind, Integer> e : counts.entrySet()) {
            int n = e.getValue();
            switch (e.getKey()) {
                case MOD:
                    parts.add(count(n, ContentKind.MODS));
                    break;
                case RESOURCE_PACK:
                    parts.add(count(n, ContentKind.RESOURCE_PACKS));
                    break;
                case SHADER_PACK:
                    parts.add(count(n, ContentKind.SHADERS));
                    break;
                case DATA_PACK:
                    parts.add(n == 1 ? GetText.tr("1 data pack") : GetText.tr("{0} data packs", n));
                    break;
                default:
                    break;
            }
        }
        return join(parts);
    }

    private static String join(List<String> parts) {
        if (parts.isEmpty()) {
            return GetText.tr("nothing");
        }
        if (parts.size() == 1) {
            return parts.get(0);
        }
        return String.join(", ", parts.subList(0, parts.size() - 1)) + " " + GetText.tr("and") + " "
            + parts.get(parts.size() - 1);
    }

    private static int count(List<DroppedFile> files, DroppedFile.Kind kind) {
        int n = 0;
        for (DroppedFile f : files) {
            if (f.kind == kind) {
                n++;
            }
        }
        return n;
    }

    /** Asked from the copying thread; the question itself is shown on the interface thread. */
    private static ContentService.Conflict askConflict(Window owner, DroppedFile file, String where) {
        final int[] answer = { -1 };
        Runnable ask = () -> answer[0] = ChoiceDialog.ask(owner, Icons.WARNING,
            GetText.tr("{0} is already in {1}", file.fileName, where),
            GetText.tr("Replace the one that's there, or keep both? Keeping both adds this one with a number after its name."),
            GetText.tr("Skip"), GetText.tr("Keep both"), GetText.tr("Replace"));
        try {
            if (SwingUtilities.isEventDispatchThread()) {
                ask.run();
            } else {
                SwingUtilities.invokeAndWait(ask);
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
        } catch (InvocationTargetException e) {
            LogManager.logStackTrace("Couldn't ask about a file", e);
        }
        switch (answer[0]) {
            case 1:
                return ContentService.Conflict.KEEP_BOTH;
            case 2:
                return ContentService.Conflict.REPLACE;
            default:
                return ContentService.Conflict.SKIP;
        }
    }

    private static void showUnknown(Window owner, List<String> names) {
        if (names.isEmpty()) {
            return;
        }
        ChoiceDialog.ask(owner, Icons.INFO,
            names.size() == 1 ? GetText.tr("{0} wasn't added", names.get(0))
                : GetText.tr("{0} files weren't added", names.size()),
            GetText.tr("Only mods, resource packs, shader packs, data packs and modpacks can be dropped here. Not added: {0}",
                String.join(", ", names)),
            GetText.tr("OK"));
    }

    // ---------------------------------------------------------------- shaders without a shader loader

    private static void offerShaderLoader(Window owner, Instance instance, ContentService service,
        Consumer<Instance> changed) {
        LoaderSupport.ShaderSetup setup = service.shaderSetup();
        if (setup == LoaderSupport.ShaderSetup.READY) {
            return;
        }
        if (setup == LoaderSupport.ShaderSetup.NEEDS_MOD_LOADER) {
            ChoiceDialog.ask(owner, Icons.SPARKLE, GetText.tr("Shaders need a mod loader"),
                GetText.tr("{0} runs plain Minecraft, which can't show shaders. Add Fabric and Iris to it from Mods and packs.",
                    instance.launcher.name),
                GetText.tr("OK"));
            return;
        }
        boolean iris = setup == LoaderSupport.ShaderSetup.ADD_IRIS;
        int answer = ChoiceDialog.ask(owner, Icons.SPARKLE,
            iris ? GetText.tr("Shaders need Iris") : GetText.tr("Shaders need Oculus"),
            iris ? GetText.tr("Minecraft can't show shaders on its own. Add Iris, with Sodium to keep the game fast?")
                : GetText.tr("On Forge, shaders need Oculus (or OptiFine from optifine.net). Add Oculus from Modrinth?"),
            GetText.tr("Not now"), iris ? GetText.tr("Add Iris and Sodium") : GetText.tr("Add Oculus"));
        if (answer != 1) {
            return;
        }
        ProgressDialog<String> dialog = new ProgressDialog<>(GetText.tr("Adding a shader loader"), 0,
            GetText.tr("Adding a shader loader"), owner);
        dialog.addThread(new Thread(() -> {
            try {
                service.installShaderLoader(new ContentService.Progress() {
                    @Override
                    public void status(String text) {
                        SwingUtilities.invokeLater(() -> dialog.setLabel(text));
                    }

                    @Override
                    public void fraction(double fraction) {
                    }
                });
            } catch (Exception e) {
                dialog.setReturnValue(e.getMessage());
            }
            dialog.close();
        }, "shader-loader"));
        dialog.start();
        if (dialog.getReturnValue() != null) {
            ChoiceDialog.ask(owner, Icons.INFO, GetText.tr("Couldn't add a shader loader"),
                dialog.getReturnValue(), GetText.tr("OK"));
        } else {
            toast(GetText.tr("Shaders are ready"));
            if (changed != null) {
                changed.accept(instance);
            }
        }
    }

    // ---------------------------------------------------------------- modpacks

    private static void offerImport(Window owner, DroppedFile pack) {
        String what;
        switch (pack.kind) {
            case MODRINTH_MODPACK:
                what = GetText.tr("a Modrinth modpack");
                break;
            case CURSEFORGE_MODPACK:
                what = GetText.tr("a CurseForge modpack");
                break;
            default:
                what = GetText.tr("an instance exported from Prism Launcher or MultiMC");
                break;
        }
        int answer = ChoiceDialog.ask(owner, Icons.IMPORT, GetText.tr("Import {0}?", pack.displayName()),
            GetText.tr("{0} is {1}. Modpacks become a new instance of their own instead of going into one.",
                pack.fileName, what),
            GetText.tr("Skip"), GetText.tr("Import as new instance"));
        if (answer == 1) {
            importModpack(owner, pack);
        }
    }

    /** The same import the Import window uses for archives. */
    private static void importModpack(Window owner, DroppedFile pack) {
        if (AccountManager.getSelectedAccount() == null) {
            ChoiceDialog.ask(owner, Icons.PERSON, GetText.tr("Add an account first"),
                GetText.tr("Instances are made for an account. Add one (an offline one is fine), then drop the modpack again."),
                GetText.tr("OK"));
            if (AccountManager.getAccounts().isEmpty()) {
                App.navigate(UIConstants.LAUNCHER_ACCOUNTS_TAB);
            }
            return;
        }
        ProgressDialog<Boolean> dialog = new ProgressDialog<>(GetText.tr("Import Instance"), 0,
            GetText.tr("Import Instance"), owner);
        dialog.addThread(new Thread(() -> {
            dialog.setReturnValue(ImportPackUtils.loadFromFile(pack.path.toFile()));
            dialog.close();
        }, "import-dropped-modpack"));
        dialog.start();
        if (!Boolean.TRUE.equals(dialog.getReturnValue())) {
            ChoiceDialog.ask(owner, Icons.WARNING, GetText.tr("Couldn't import {0}", pack.displayName()),
                GetText.tr("The file couldn't be read as a modpack. The launcher log has the details."),
                GetText.tr("OK"));
        }
    }

    private static void toast(String text) {
        if (App.TOASTER != null) {
            App.TOASTER.pop(text);
        }
    }
}
