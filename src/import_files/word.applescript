on run argv
    set inputFile to POSIX file (item 1 of argv) as alias
    set outputFile to POSIX file (item 2 of argv) as text
    set openedDocument to missing value
    tell application "Microsoft Word"
        set previousSecurity to automation security
        try
            set automation security to msoAutomationSecurityForceDisable
            set openedDocument to open inputFile read only true add to recent files false confirm conversions false
            if class of openedDocument is list then set openedDocument to item 1 of openedDocument
            save as openedDocument file name outputFile file format format PDF add to recent files false
            close openedDocument saving no
            set openedDocument to missing value
            set automation security to previousSecurity
        on error errorMessage number errorNumber
            if openedDocument is not missing value then
                try
                    close openedDocument saving no
                end try
            end if
            set automation security to previousSecurity
            error errorMessage number errorNumber
        end try
    end tell
end run
