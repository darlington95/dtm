import { Box, Button, chakra, Em, HStack, Textarea, VStack } from "@chakra-ui/react"
import { useEffect, useRef, useState } from "react"
import { PanelButton, Tooltip } from "@/components"
import IconToggle from "@/components/IconToggle"
import { PiInfo } from "@/components/icons/icons"
import { CLOSE_TRANSIENT_POPUPS_EVENT } from "@/dtProjects/imagesList/ContentPanelPopup"
import TabContent from "@/components/TabContent"
import { useDTP } from "../state/context"
import SearchFilterForm from "./filters/SearchFilterForm"

interface SearchPanelComponentProps extends ChakraProps {}

function SearchPanel(props: SearchPanelComponentProps) {
    const { ...restProps } = props
    const { search, uiState } = useDTP()

    const searchSnap = search.useSnap()
    const { shouldFocus } = uiState.useSnap()

    const [searchInput, setSearchInput] = useState("")
    const searchInputRef = useRef<HTMLTextAreaElement>(null)

    useEffect(() => {
        if (searchInputRef.current && shouldFocus === "searchInput") {
            searchInputRef.current.focus()
        }
    }, [shouldFocus])

    const searchMode = searchSnap.searchMode ?? "auto"

    return (
        <TabContent
            value={"search"}
            overflowX={"clip"}
            height={"auto"}
            padding={2}
            justifyContent={"flex-start"}
            onKeyDown={(e) => {
                if (e.key === "Enter") {
                    window.dispatchEvent(new Event(CLOSE_TRANSIENT_POPUPS_EVENT))
                    search.applySearch()
                }
            }}
            {...restProps}
        >
            <HStack>
                <Textarea
                    ref={searchInputRef}
                    data-defctx={true}
                    rows={1}
                    autoresize
                    bgColor={"bg.3"}
                    value={searchInput}
                    onChange={(e) => {
                        const value = e.target.value.replace(/\n/g, "")
                        setSearchInput(value)
                        search.state.searchInput = value
                    }}
                    border={"1px solid"}
                    borderColor={"grayc.13"}
                    borderRadius={"lg"}
                    placeholder="Search"
                    width={"full"}
                />
                <Tooltip tip={<SearchInfo />}>
                    <PiInfo size={16} />
                </Tooltip>
            </HStack>

            {/* 検索モード切替: 語マッチ / 文字列の部分一致(CJK 対応) */}
            <HStack marginTop={2} gap={2} alignItems={"center"}>
                <Tooltip
                    tip={
                        <Box>
                            <Em>Auto</Em> picks the matching strategy by the query: word matching
                            for ASCII / Latin text, substring matching when any other script
                            (CJK, Cyrillic, etc.) is included. <Em>Word</Em> matches individual
                            words with stemming (good for English). <Em>Text</Em> matches
                            substrings, so it also works for CJK languages like Japanese / Chinese
                            where words are not space-separated.
                        </Box>
                    }
                >
                    <Box fontSize={"sm"} color={"fg.2"}>
                        Match
                    </Box>
                </Tooltip>
                <IconToggle
                    value={{
                        auto: searchMode === "auto",
                        word: searchMode === "word",
                        substring: searchMode === "substring",
                    }}
                    onChange={(value) => {
                        search.setSearchMode(value.auto ? "auto" : value.word ? "word" : "substring")
                    }}
                    mode={"requireOne"}
                >
                    <IconToggle.Trigger
                        aria-label={"Auto match"}
                        size={"sm"}
                        variant={"ghost"}
                        option={"auto"}
                        tipText={
                            "Auto: word matching for ASCII / Latin text, substring matching otherwise"
                        }
                    >
                        Auto
                    </IconToggle.Trigger>
                    <IconToggle.Trigger
                        aria-label={"Match by word"}
                        size={"sm"}
                        variant={"ghost"}
                        option={"word"}
                        tipText={"Word matching (stemming)"}
                    >
                        Word
                    </IconToggle.Trigger>
                    <IconToggle.Trigger
                        aria-label={"Match by substring"}
                        size={"sm"}
                        variant={"ghost"}
                        option={"substring"}
                        tipText={"Substring matching (CJK-friendly)"}
                    >
                        Text
                    </IconToggle.Trigger>
                </IconToggle>
            </HStack>

            <VStack
                display={searchSnap.filters.length > 0 ? "flex" : "none"}
                gap={0.5}
                bgColor={"grayc.13"}
                // border={"1px solid"}
                // borderColor={"grayc.11"}
                // _dark={{ borderColor: "grayc.15" }}
                width={"100%"}
                alignItems={"stretch"}
                paddingY={0.5}
                paddingX={0.5}
                marginY={2}
                borderRadius={"lg"}
                // boxShadow={"0px 1px 8px -4px #00000033"}
            >
                {searchSnap.filters.map((filter) => (
                    <SearchFilterForm
                        index={filter.index}
                        key={filter.index}
                        onRemove={() => search.removeFilter(filter.index)}
                    />
                ))}
            </VStack>
            <Button
                aria-label="Add search filter"
                unstyled
                width={"max-content"}
                height={"auto"}
                marginX={"auto"}
                padding={2}
                color={"fg.2"}
                _hover={{ textDecoration: "underline", color: "fg.1" }}
                fontSize={"sm"}
                // cursor={"pointer"}
                onClick={() => search.addEmptyFilter(true)}
            >
                Add Filter
            </Button>
            <HStack marginTop={4} width={"full"} gap={0}>
                <PanelButton
                    aria-label="Reset search"
                    borderLeftRadius={"lg"}
                    borderRightRadius={"none"}
                    boxShadow={"xs"}
                    flex={"0 0 auto"}
                    onClick={() => {
                        setSearchInput("")
                        search.state.searchInput = ""
                        search.clearFilters()
                    }}
                >
                    Reset
                </PanelButton>
                <PanelButton
                    aria-label="Apply search"
                    borderLeftRadius={"none"}
                    borderRightRadius={"lg"}
                    boxShadow={"xs"}
                    flex={"1 1 auto"}
                    onClick={() => {
                        window.dispatchEvent(new Event(CLOSE_TRANSIENT_POPUPS_EVENT))
                        search.applySearch()
                    }}
                >
                    Search
                </PanelButton>
            </HStack>
        </TabContent>
    )
}

function SearchInfo() {
    return (
        <VStack fontSize={"sm"} alignItems={"flex-start"}>
            <Box>Images will match if the prompt contains any of the search terms.</Box>
            <Box>
                <Em>Auto</Em> mode chooses by the query: ASCII / Latin text uses word matching,
                and any other script (Japanese / Chinese / Cyrillic, etc.) uses substring
                matching.
            </Box>
            <Box>
                <Em>Word</Em> mode stems terms, so <Em>shade</Em>, <Em>shades</Em>, <Em>shading</Em>,
                and <Em>shaded</Em> are all seen as the same word.
            </Box>
            <Box>
                <Em>Text</Em> (substring) mode matches any contiguous substring, so it works for CJK
                languages like Japanese / Chinese where words are not separated by spaces.
            </Box>
            <Box>Wrap words or phrases in "quotes" to require an exact text match.</Box>
            <B>cow boy</B>
            <Box marginTop={-2}>
                Matches images that have the words <Em>cow</Em> or <Em>boy</Em> in the prompt - but
                not <Em>cowboy</Em> since that is a different word
            </Box>
            <B>"cow" "boy"</B>
            <Box marginTop={-2}>
                Matches images that have both <Em>cow</Em> and <Em>boy</Em> in the prompt -
                including <Em>cowboys</Em>
            </Box>
            <B>"cow boy"</B>
            <Box marginTop={-2}>
                Matches images that have the exact phrase <Em>cow boy</Em> in the prompt
            </Box>
        </VStack>
    )
}

const B = chakra("span", {
    base: {
        fontWeight: "bold",
        marginBottom: "0",
    },
})

export default SearchPanel
