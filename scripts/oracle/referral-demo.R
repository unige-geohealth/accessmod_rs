# Runs inside the pinned AccessMod 5 image (see export.sh): the demo referral
# replay (replay/examples/demo_referral, units set to seconds and metres) with
# 8 and 16 neighbours. Writes the referral tables as CSV to
# /out/referral/demo/oracle/.
source("global.R")
out <- "/out/referral/demo/oracle"
dir.create(out, recursive = TRUE, showWarnings = FALSE)
db <- "/data/dbgrass/demo/demo/sqlite.db"

for (knight in c(FALSE, TRUE)) {
  name <- if (knight) "16" else "8"
  conf <- amAnalysisReplayParseConf("/scripts/referral-demo.json")
  conf$args$knightMove <- knight
  conf$args$outputReferral <- paste0("tReferral__oracle", name)
  conf$args$outputNearestDist <- paste0("tReferralDist__oracle", name)
  conf$args$outputNearestTime <- paste0("tReferralTime__oracle", name)
  conf$args$outputNetDist <- paste0("vReferralNetwork__oracle", name)
  started <- Sys.time()
  amAnalysisReplayExec(conf)
  elapsed <- as.numeric(Sys.time() - started, units = "secs")
  con <- DBI::dbConnect(RSQLite::SQLite(), db)
  for (table in c("tReferral", "tReferralDist", "tReferralTime")) {
    data <- DBI::dbReadTable(con, paste0(table, "__oracle", name))
    write.csv(data, file.path(out, sprintf("%s-%s.csv", table, name)), row.names = FALSE)
  }
  DBI::dbDisconnect(con)
  writeLines(sprintf("%.3f", elapsed), file.path(out, sprintf("seconds-%s.log", name)))
}
