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
package com.blockforge.launcher.utils;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.DirectoryStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.util.Optional;
import java.util.zip.GZIPInputStream;

/**
 * Singleplayer worlds in a game folder: which one was played last, and its name as shown in the game.
 */
public final class Worlds {
    /** level.dat is a few KB; anything this big isn't one */
    private static final int MAX_LEVEL_DAT = 8 * 1024 * 1024;

    private Worlds() {
    }

    /** One world. */
    public static final class World {
        /** the folder in saves, which is what Quick Play needs */
        public final String folder;
        /** the name shown in the game, or the folder name when it can't be read */
        public final String name;
        public final FileTime lastPlayed;

        World(String folder, String name, FileTime lastPlayed) {
            this.folder = folder;
            this.name = name;
            this.lastPlayed = lastPlayed;
        }
    }

    /**
     * The world played most recently: the one whose level.dat was written last (the game writes it on every save and
     * when leaving the world).
     */
    public static Optional<World> latest(Path savesDirectory) {
        if (savesDirectory == null || !Files.isDirectory(savesDirectory)) {
            return Optional.empty();
        }

        Path best = null;
        FileTime bestTime = null;
        try (DirectoryStream<Path> folders = Files.newDirectoryStream(savesDirectory)) {
            for (Path folder : folders) {
                Path levelDat = folder.resolve("level.dat");
                if (!Files.isRegularFile(levelDat)) {
                    continue;
                }
                FileTime time = Files.getLastModifiedTime(levelDat);
                if (bestTime == null || time.compareTo(bestTime) > 0
                    || (time.compareTo(bestTime) == 0 && folder.getFileName().toString()
                        .compareTo(best.getFileName().toString()) < 0)) {
                    best = folder;
                    bestTime = time;
                }
            }
        } catch (IOException e) {
            return Optional.empty();
        }

        if (best == null) {
            return Optional.empty();
        }

        String folderName = best.getFileName().toString();
        String name = readLevelName(best.resolve("level.dat"));
        return Optional.of(new World(folderName, name == null || name.trim().isEmpty() ? folderName : name.trim(),
            bestTime));
    }

    /**
     * The world's name from its level.dat (gzipped NBT; Data.LevelName), or null. Only looks for the one string
     * tag, without reading the whole structure.
     */
    public static String readLevelName(Path levelDat) {
        try (InputStream in = new GZIPInputStream(Files.newInputStream(levelDat))) {
            byte[] data = readLimited(in);
            return findStringTag(data, "LevelName");
        } catch (IOException | RuntimeException e) {
            return null;
        }
    }

    private static byte[] readLimited(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[8192];
        int n;
        while ((n = in.read(buffer)) > 0) {
            out.write(buffer, 0, n);
            if (out.size() > MAX_LEVEL_DAT) {
                throw new IOException("too big for a level.dat");
            }
        }
        return out.toByteArray();
    }

    /**
     * Finds a string tag by name in uncompressed NBT: type byte 8, a two byte name length, the name, then a two byte
     * value length and the value (Java's modified UTF-8, which is plain UTF-8 for ordinary text).
     */
    static String findStringTag(byte[] data, String tagName) {
        byte[] name = tagName.getBytes(StandardCharsets.UTF_8);
        outer: for (int i = 0; i + 3 + name.length + 2 <= data.length; i++) {
            if (data[i] != 8 || data[i + 1] != (byte) (name.length >> 8) || data[i + 2] != (byte) name.length) {
                continue;
            }
            for (int j = 0; j < name.length; j++) {
                if (data[i + 3 + j] != name[j]) {
                    continue outer;
                }
            }
            int at = i + 3 + name.length;
            int length = ((data[at] & 0xFF) << 8) | (data[at + 1] & 0xFF);
            if (at + 2 + length > data.length) {
                return null;
            }
            return new String(data, at + 2, length, StandardCharsets.UTF_8);
        }
        return null;
    }
}
