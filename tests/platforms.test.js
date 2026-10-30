import test from "node:test";
import assert from "node:assert/strict";
import { projectId, timestamp, beijingInput, normalizeBili, biliScreens, normalizeDamai, normalizeDamaiTickets, normalizeMotianlun, normalizeMotianlunTickets, validateSelection, maskId } from "../src/services/platforms.js";

test("project links only accept the selected platform and exact query keys", () => {
    assert.equal(projectId("https://m.damai.cn/damai/detail/item.html?itemId=123&spm=x", "dm"), "123");
    assert.equal(projectId("https://mall.bilibili.com/neul-next/ticket-renovation/detail.html?otherid=456&id=123", "bilibili"), "123");
    assert.equal(projectId("https://show.bilibili.com/platform/detail.html?otherid=456", "bilibili"), "");
    assert.equal(projectId("https://bilibili.com.example.org/?id=123", "bilibili"), "");
    assert.equal(projectId("https://m.damai.cn/?itemId=123", "bilibili"), "");
    assert.equal(projectId(" 123456 ", "dm"), "123456");
    assert.equal(projectId("69FB10C1B0BCC10001A27774", "motianlun"), "69fb10c1b0bcc10001a27774");
    assert.equal(projectId("https://www.motianlun.cn/content/69fb10c1b0bcc10001a27774", "motianlun"), "69fb10c1b0bcc10001a27774");
    assert.equal(projectId("https://m-mtl-global.moretickets.com/tourDetail?showId=69fb10c1b0bcc10001a27774", "motianlun"), "69fb10c1b0bcc10001a27774");
    assert.equal(projectId("https://www.moretickets.com/show-detail?showId=69fb10c1b0bcc10001a27774", "motianlun"), "69fb10c1b0bcc10001a27774");
    assert.equal(projectId("https://example.com/content/69fb10c1b0bcc10001a27774", "motianlun"), "");
    assert.equal(projectId("https://m.damai.cn/?itemId=123", "motianlun"), "");
});

test("Motianlun sessions and inventory keep stock and price", () => {
    const project = normalizeMotianlun({
        note: "已加载",
        show: { id: "69fb10c1b0bcc10001a27774", showName: "演出", cityName: "香港", venueName: "主场馆", posterUrl: "http://img.example/a.png" },
        sessions: [{ sessionId: "6a016a61bb59430001084250", sessionName: "周五场", sessionStatus: "ONSALE", hasTicket: true, purchaseLimit: 4 }],
    }, "69fb10c1b0bcc10001a27774");
    assert.equal(project.name, "演出");
    assert.equal(project.image.startsWith("https:"), true);
    assert.equal(project.screens[0].disabled, false);
    assert.equal(project.screens[0].limit, 4);
    const tickets = normalizeMotianlunTickets([{ inventoryId: "6ae3a9835983470001717b08", originPrice: "2000", currencySymbol: "HK$", stock: 2, mtsCategoryName: "内场", rowName: "B行" }], 4);
    assert.equal(tickets[0].price, 200000);
    assert.equal(tickets[0].currency, "HK$");
    assert.equal(tickets[0].limit, 2);
    assert.equal(tickets[0].disabled, false);
    assert.equal(normalizeMotianlunTickets([{ inventoryId: "6ae44b6fc5ca3d0001dbfc07", originPrice: "10", stock: 0 }])[0].disabled, true);
});

test("sale times use Beijing timezone, independent of the system timezone", () => {
    const utc = Date.UTC(2026, 8, 5, 4, 30, 0);
    assert.equal(timestamp("2026-09-05 12:30:00"), utc);
    assert.equal(timestamp("2026-09-05T12:30:00"), utc);
    assert.equal(timestamp("2026-09-05T04:30:00Z"), utc);
    assert.equal(timestamp(utc / 1000), utc);
    assert.equal(timestamp(utc), utc);
    assert.equal(timestamp("invalid"), 0);
    assert.equal(timestamp(beijingInput(utc)), utc);
});

test("both Bilibili detail formats produce consistent selectable tickets", () => {
    const old = normalizeBili({ id: 123, name: "演出", screen_list: [{ id: 10, name: "第一场", express_fee: 1200, ticket_list: [{ id: 20, desc: "普通票", price: 8800, static_limit: { num: 2 }, sale_start: 1788582600 }] }] });
    const current = normalizeBili({ projectId: 123, projectName: "演出", screenList: [{ screenId: 10, screenName: "第一场", expressFee: 1200, ticketList: [{ skuId: 20, skuName: "普通票", ticketPrice: 8800, staticLimit: { num: 2 }, saleStart: 1788582600000 }] }] });
    assert.deepEqual(old.screens, current.screens);
    assert.equal(current.screens[0].tickets[0].price, 10000);
    assert.equal(current.screens[0].tickets[0].limit, 2);
    assert.equal(current.screens[0].tickets[0].saleStart, 1788582600000);
});

test("Bilibili keeps screen and ticket availability from both detail APIs", () => {
    const old = biliScreens([{ id: 1, clickable: false, sale_flag: { display_name: "停售" }, ticket_list: [{ id: 2, clickable: false, sale_flag: { display_name: "售罄" } }] }]);
    const current = biliScreens([{ screenId: 1, canClick: false, saleFlag: { displayName: "停售" }, ticketList: [{ skuId: 2, canClick: false, saleFlag: { displayName: "售罄" } }] }]);
    assert.deepEqual(old, current);
    assert.equal(current[0].disabled, true);
    assert.equal(current[0].disabledReason, "停售");
    assert.equal(current[0].tickets[0].disabled, true);
    assert.equal(current[0].tickets[0].disabledReason, "售罄");
});

test("Bilibili uses explicit clickability before sale status and preserves selectable presales", () => {
    for (const key of ["clickable", "canClick"]) {
        for (const value of [false, 0, "0", "false"]) {
            assert.equal(biliScreens([{ [key]: value }])[0].disabled, true);
        }
        for (const value of [true, 1, "1", "true"]) {
            assert.equal(biliScreens([{ [key]: value, saleFlagNumber: 101 }])[0].disabled, false);
        }
    }
    const [screen] = biliScreens([{ ticketList: [
        { sale_flag_number: 4 }, { saleFlag: { number: 105 } }, { saleFlagNumber: 6 }, {}, { clickable: false, sale_flag: "无购买资格" },
    ] }]);
    assert.equal(screen.disabled, false);
    assert.deepEqual(screen.tickets.map(ticket => ticket.disabled), [true, true, false, false, true]);
    assert.deepEqual(screen.tickets.map(ticket => ticket.status), ["售罄", "下架", "库存紧张", "", "无购买资格"]);
});

test("Damai retains all performances instead of only the first", () => {
    const project = normalizeDamai({ item: { performBases: [{ name: "周末", performs: [{ performId: 1 }, { performId: 2, performName: "晚场" }] }] } }, "99");
    assert.deepEqual(project.screens.map(screen => screen.id), ["1", "2"]);
    const tickets = normalizeDamaiTickets({ itemBasicInfo: { t: "key" }, perform: { skuList: [{ skuId: 1, price: "99.90", limitQuantity: 2 }] } });
    assert.equal(tickets[0].price, 9990);
    assert.equal(tickets[0].signKey, "key");
});

const selection = () => ({ platform: "bilibili", project: {}, screen: {}, ticket: { limit: 2 }, count: 1, buyers: ["a"], buyer: "测试联系人", tel: "13800000000", scheduled: false });

test("purchase validation rejects mismatched, duplicate and excess buyers", () => {
    assert.equal(validateSelection(selection()), "");
    assert.match(validateSelection({ ...selection(), buyers: [] }), /张数/);
    assert.match(validateSelection({ ...selection(), count: 2, buyers: ["a", "a"] }), /重复/);
    assert.match(validateSelection({ ...selection(), count: 3, buyers: ["a", "b", "c"] }), /限购/);
    assert.match(validateSelection({ ...selection(), count: 1.5 }), /限购/);
    assert.match(validateSelection({ ...selection(), screen: { requiresSeat: true } }), /选座/);
});

test("purchase validation rejects unavailable Bilibili screens and tickets", () => {
    const [screen] = biliScreens([{ canClick: false, saleFlagNumber: 3, ticketList: [{ canClick: true }] }]);
    assert.match(validateSelection({ ...selection(), screen, ticket: screen.tickets[0] }), /场次不可购买.*停售/);
    const [availableScreen] = biliScreens([{ canClick: true, ticketList: [{ canClick: false, saleFlagNumber: 4 }] }]);
    assert.match(validateSelection({ ...selection(), screen: availableScreen, ticket: availableScreen.tickets[0] }), /票档不可购买.*售罄/);
});

test("delivery and scheduling require complete information", () => {
    assert.match(validateSelection({ ...selection(), screen: { deliveryFee: 1200 } }), /收货地址/);
    assert.match(validateSelection({ ...selection(), scheduled: true, startAt: "" }), /预约时间/);
    assert.match(validateSelection({ ...selection(), tel: "abc" }), /电话/);
    assert.equal(validateSelection({ ...selection(), screen: { deliveryFee: 1200 }, address: { id: 1 } }), "");
});

test("identity numbers are masked for display", () => {
    assert.equal(maskId("110101199001011234"), "110********1234");
    assert.equal(maskId(null), "");
});
