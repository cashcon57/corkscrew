//! Built-in baseline file lists for known games.
//!
//! When no user-created snapshot exists, the game directory cleaner can fall
//! back to these hardcoded baselines to identify non-stock files. This avoids
//! requiring the user to "run the game once" before the cleaner works.
//!
//! The lists include both the base game files and all DLC/AE content so that
//! stock files are never flagged as non-stock regardless of which edition the
//! user owns.

use std::collections::HashSet;

/// Returns a built-in baseline for the given game, or `None` if no baseline
/// is available. The returned set contains relative paths (forward-slash
/// separated) that are considered stock files in the game's Data directory.
pub fn get_builtin_baseline(game_id: &str) -> Option<HashSet<String>> {
    match game_id {
        "skyrimse" => Some(skyrim_se_baseline()),
        _ => None,
    }
}

/// Returns `true` if the given relative path matches a known stock pattern
/// for the game, even if it's not in the explicit baseline list.
/// This catches files like Creation Club content (cc*.esl/esm/bsa) that have
/// many variants.
pub fn is_stock_pattern(game_id: &str, rel_path: &str) -> bool {
    match game_id {
        "skyrimse" => is_skyrim_stock_pattern(rel_path),
        _ => false,
    }
}

/// Skyrim SE stock pattern matching for files not in the explicit list.
fn is_skyrim_stock_pattern(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();

    // Creation Club content: cc*.esm, cc*.esl, cc*.bsa
    if lower.starts_with("cc")
        && (lower.ends_with(".esm") || lower.ends_with(".esl") || lower.ends_with(".bsa"))
    {
        return true;
    }

    // _ResourcePack (AE resource pack)
    if lower.starts_with("_resourcepack.") {
        return true;
    }

    // Video files are stock
    if lower.starts_with("video/") || lower.starts_with("video\\") {
        return true;
    }

    // Localized voice BSAs: Skyrim - Voices_XX0.bsa (any language)
    if lower.starts_with("skyrim - voices_") && lower.ends_with("0.bsa") {
        return true;
    }

    false
}

/// Skyrim Special Edition / Anniversary Edition stock Data directory files.
///
/// This list covers:
/// - Base game ESMs + BSAs
/// - Official DLC (Dawnguard, HearthFires, Dragonborn)
/// - Anniversary Edition Creation Club content
/// - Video files
///
/// The list is intentionally comprehensive — including AE files doesn't hurt
/// SE-only users (those files simply won't exist on disk).
fn skyrim_se_baseline() -> HashSet<String> {
    let files: &[&str] = &[
        // === Base game ===
        "Skyrim.esm",
        "Update.esm",
        "Skyrim - Animations.bsa",
        "Skyrim - Interface.bsa",
        "Skyrim - Meshes0.bsa",
        "Skyrim - Meshes1.bsa",
        "Skyrim - Misc.bsa",
        "Skyrim - Shaders.bsa",
        "Skyrim - Sounds.bsa",
        "Skyrim - Textures0.bsa",
        "Skyrim - Textures1.bsa",
        "Skyrim - Textures2.bsa",
        "Skyrim - Textures3.bsa",
        "Skyrim - Textures4.bsa",
        "Skyrim - Textures5.bsa",
        "Skyrim - Textures6.bsa",
        "Skyrim - Textures7.bsa",
        "Skyrim - Textures8.bsa",
        "Skyrim - Patch.bsa",
        // Localized voice files (common languages)
        "Skyrim - Voices_en0.bsa",
        "Skyrim - Voices_fr0.bsa",
        "Skyrim - Voices_de0.bsa",
        "Skyrim - Voices_it0.bsa",
        "Skyrim - Voices_es0.bsa",
        "Skyrim - Voices_ja0.bsa",
        "Skyrim - Voices_pl0.bsa",
        "Skyrim - Voices_ru0.bsa",
        // === DLC: Dawnguard ===
        "Dawnguard.esm",
        "Dawnguard.bsa",
        // === DLC: HearthFires ===
        "HearthFires.esm",
        "HearthFires.bsa",
        // === DLC: Dragonborn ===
        "Dragonborn.esm",
        "Dragonborn.bsa",
        // === Anniversary Edition resource pack ===
        "_ResourcePack.esl",
        "_ResourcePack.bsa",
        // === AE Creation Club content ===
        // Saints & Seducers
        "ccBGSSSE025-AdvDSGS.esm",
        "ccBGSSSE025-AdvDSGS.bsa",
        // Rare Curios
        "ccBGSSSE037-Curios.esl",
        "ccBGSSSE037-Curios.bsa",
        // Survival Mode
        "ccQDRSSE001-SurvivalMode.esl",
        "ccQDRSSE001-SurvivalMode.bsa",
        // Fishing
        "ccBGSSSE001-Fish.esm",
        "ccBGSSSE001-Fish.bsa",
        // Pets of Skyrim
        "ccVSVSSE002-Pets.esl",
        "ccVSVSSE002-Pets.bsa",
        // Camping
        "ccBGSSSE068-Embers.esl",
        "ccBGSSSE068-Embers.bsa",
        // Dead Man's Dread
        "ccEEJSSE005-Cave.esm",
        "ccEEJSSE005-Cave.bsa",
        // Ghosts of the Tribunal
        "ccEEJSSE001-Hstead.esm",
        "ccEEJSSE001-Hstead.bsa",
        // The Cause
        "ccEEJSSE002-Tower.esl",
        "ccEEJSSE002-Tower.bsa",
        // Forgotten Seasons
        "ccEEJSSE003-Hollow.esl",
        "ccEEJSSE003-Hollow.bsa",
        // Hendraheim
        "ccEEJSSE004-Hall.esl",
        "ccEEJSSE004-Hall.bsa",
        // Necromantic Grimoire
        "ccVSVSSE003-NecroArts.esl",
        "ccVSVSSE003-NecroArts.bsa",
        // Farming
        "ccVSVSSE004-BeAFarmer.esl",
        "ccVSVSSE004-BeAFarmer.bsa",
        // Winter
        "ccVSVSSE001-Winter.esl",
        "ccVSVSSE001-Winter.bsa",
        // Puzzle Dungeon
        "ccTWBSSE001-PuzzleDungeon.esm",
        "ccTWBSSE001-PuzzleDungeon.bsa",
        // Necro House
        "ccRMSSSE001-NecroHouse.esl",
        "ccRMSSSE001-NecroHouse.bsa",
        // Staves
        "ccBGSSSE066-Staves.esl",
        "ccBGSSSE066-Staves.bsa",
        // Daedric Inventory
        "ccBGSSSE067-DaedInv.esm",
        "ccBGSSSE067-DaedInv.bsa",
        // Contest
        "ccBGSSSE069-Contest.esl",
        "ccBGSSSE069-Contest.bsa",
        // Mount Floor
        "ccBGSSSE034-MntFlr.esl",
        "ccBGSSSE034-MntFlr.bsa",
        // Hasedoki
        "ccBGSSSE045-Hasedoki.esl",
        "ccBGSSSE045-Hasedoki.bsa",
        // Arms of Chaos
        "ccPEWSSE002-ArmsOfChaos.esl",
        "ccPEWSSE002-ArmsOfChaos.bsa",
        // Crossbow Pack
        "ccFFBSSE002-CrossbowPack.esl",
        "ccFFBSSE002-CrossbowPack.bsa",
        // Imperial Dragon
        "ccFFBSSE001-ImperialDragon.esl",
        "ccFFBSSE001-ImperialDragon.bsa",
        // Knights Pack
        "ccMTYSSE001-KnightsPack.esl",
        "ccMTYSSE001-KnightsPack.bsa",
        // Vampire Elders
        "ccMTYSSE002-VE.esl",
        "ccMTYSSE002-VE.bsa",
        // Altar
        "ccKRTSSE001_Altar.esl",
        "ccKRTSSE001_Altar.bsa",
        // Firewood
        "ccQDRSSE002-Firewood.esl",
        "ccQDRSSE002-Firewood.bsa",
        // Bone Wolf (Alternative Armors)
        "ccBGSSSE050-BA_Daedric.esl",
        "ccBGSSSE050-BA_Daedric.bsa",
        "ccBGSSSE051-BA_DaedricMail.esl",
        "ccBGSSSE051-BA_DaedricMail.bsa",
        "ccBGSSSE052-BA_Iron.esl",
        "ccBGSSSE052-BA_Iron.bsa",
        "ccBGSSSE053-BA_Leather.esl",
        "ccBGSSSE053-BA_Leather.bsa",
        "ccBGSSSE054-BA_Orcish.esl",
        "ccBGSSSE054-BA_Orcish.bsa",
        "ccBGSSSE055-BA_OrcishScaled.esl",
        "ccBGSSSE055-BA_OrcishScaled.bsa",
        "ccBGSSSE057-BA_Stalhrim.esl",
        "ccBGSSSE057-BA_Stalhrim.bsa",
        "ccBGSSSE058-BA_Steel.esl",
        "ccBGSSSE058-BA_Steel.bsa",
        "ccBGSSSE059-BA_Dragonplate.esl",
        "ccBGSSSE059-BA_Dragonplate.bsa",
        "ccBGSSSE060-BA_Dragonscale.esl",
        "ccBGSSSE060-BA_Dragonscale.bsa",
        "ccBGSSSE061-BA_Dwarven.esl",
        "ccBGSSSE061-BA_Dwarven.bsa",
        "ccBGSSSE062-BA_DwarvenMail.esl",
        "ccBGSSSE062-BA_DwarvenMail.bsa",
        "ccBGSSSE063-BA_Ebony.esl",
        "ccBGSSSE063-BA_Ebony.bsa",
        "ccBGSSSE064-BA_Elven.esl",
        "ccBGSSSE064-BA_Elven.bsa",
        // === Video files ===
        "Video/BGS_Logo.bik",
    ];

    files.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skyrim_baseline_not_empty() {
        let baseline = get_builtin_baseline("skyrimse").unwrap();
        assert!(baseline.len() > 30); // At least base game + DLC
        assert!(baseline.contains("Skyrim.esm"));
        assert!(baseline.contains("Dawnguard.esm"));
        assert!(baseline.contains("Dragonborn.esm"));
    }

    #[test]
    fn unknown_game_returns_none() {
        assert!(get_builtin_baseline("unknowngame").is_none());
    }

    #[test]
    fn stock_patterns_work() {
        assert!(is_stock_pattern("skyrimse", "ccNewMod001-Test.esl"));
        assert!(is_stock_pattern("skyrimse", "ccNewMod001-Test.bsa"));
        assert!(is_stock_pattern("skyrimse", "Video/intro.bik"));
        assert!(is_stock_pattern("skyrimse", "Skyrim - Voices_pt0.bsa"));
        assert!(is_stock_pattern("skyrimse", "_ResourcePack.esl"));
        // Non-stock patterns
        assert!(!is_stock_pattern("skyrimse", "mod.esp"));
        assert!(!is_stock_pattern("skyrimse", "SKSE/Plugins/test.dll"));
    }
}
