from typing import Any, Dict, Optional


def wso_record_ingest_args(
    record: Dict[str, Any], scraper_secret: Optional[str] = None
) -> Dict[str, Any]:
    args: Dict[str, Any] = {
        "wso": record["wso"],
        "ageCategory": record["age_category"],
        "gender": record["gender"],
        "weightClass": record["weight_class"],
    }
    if scraper_secret:
        args["scraperSecret"] = scraper_secret
    if record.get("snatch_record") is not None:
        args["snatchRecord"] = record["snatch_record"]
    if record.get("cj_record") is not None:
        args["cjRecord"] = record["cj_record"]
    if record.get("total_record") is not None:
        args["totalRecord"] = record["total_record"]
    return args
